//! Bounded FlashWriter serial channel (adapted from edgecontroller).

use anyhow::{anyhow, bail, Context, Result};
use serialport::SerialPort;
use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    time::{Duration, Instant},
};

pub fn open_uart(path: &str, baud: u32) -> Result<Box<dyn SerialPort>> {
    serialport::new(path, baud)
        .data_bits(serialport::DataBits::Eight)
        .parity(serialport::Parity::None)
        .stop_bits(serialport::StopBits::One)
        .flow_control(serialport::FlowControl::None)
        .timeout(Duration::from_millis(500))
        .open()
        .with_context(|| format!("opening UART {path} at {baud} baud"))
}

/// Append-only device RX log (host markers + UART bytes from the board).
pub fn open_rx_log(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening UART RX log {}", path.display()))
}

pub struct Console {
    port: Box<dyn SerialPort>,
    pending: Vec<u8>,
    rx_log: Option<File>,
    /// When true, device RX bytes are appended to `rx_log`.
    rx_log_enabled: bool,
}

impl Console {
    pub fn open(path: &str, baud: u32) -> Result<Self> {
        Ok(Self {
            port: open_uart(path, baud)?,
            pending: Vec::with_capacity(4096),
            rx_log: None,
            rx_log_enabled: false,
        })
    }

    pub fn attach_rx_log(&mut self, file: File) {
        self.rx_log = Some(file);
    }

    pub fn set_rx_logging(&mut self, enabled: bool) {
        self.rx_log_enabled = enabled;
    }

    pub fn log_marker(&mut self, message: &str) -> Result<()> {
        if let Some(log) = self.rx_log.as_mut() {
            writeln!(log, "\n===== {message} =====")?;
            log.flush()?;
        }
        Ok(())
    }

    fn log_rx(&mut self, bytes: &[u8]) -> Result<()> {
        if !self.rx_log_enabled || bytes.is_empty() {
            return Ok(());
        }
        if let Some(log) = self.rx_log.as_mut() {
            log.write_all(bytes)?;
            log.flush()?;
        }
        Ok(())
    }

    pub fn clear_input(&mut self) -> Result<()> {
        self.port
            .clear(serialport::ClearBuffer::Input)
            .map_err(|error| anyhow!("clearing UART input failed: {error}"))?;
        self.pending.clear();
        Ok(())
    }

    /// Pull any queued RX bytes into `pending` without blocking long.
    ///
    /// Must run during large S-record uploads: otherwise the kernel tty
    /// buffer can overrun and the FlashWriter `>` prompt is lost.
    fn drain_rx(&mut self) -> Result<()> {
        let available = self.port.bytes_to_read().unwrap_or(0);
        if available == 0 {
            return Ok(());
        }

        let mut buf = vec![0_u8; available.min(4096) as usize];
        match self.port.read(&mut buf) {
            Ok(0) => Ok(()),
            Ok(count) => {
                self.log_rx(&buf[..count])?;
                self.pending.extend_from_slice(&buf[..count]);
                if self.pending.len() > 16_384 {
                    let discard = self.pending.len() - 4096;
                    self.pending.drain(..discard);
                }
                Ok(())
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                ) =>
            {
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Recent UART text for timeout diagnostics.
    pub fn pending_preview(&self) -> String {
        let tail = if self.pending.len() > 400 {
            &self.pending[self.pending.len() - 400..]
        } else {
            &self.pending[..]
        };
        String::from_utf8_lossy(tail)
            .chars()
            .map(|ch| {
                if ch == '\r' || ch == '\n' || ch.is_ascii_graphic() || ch == ' ' {
                    ch
                } else {
                    '·'
                }
            })
            .collect()
    }

    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        self.port
            .write_all(bytes)
            .map_err(|error| anyhow!("UART write failed: {error}"))?;
        self.port
            .flush()
            .map_err(|error| anyhow!("UART flush failed: {error}"))?;
        Ok(())
    }

    pub fn command(&mut self, command: &str) -> Result<()> {
        self.send(command.as_bytes())?;
        // Preserve command pacing expected by FlashWriter.
        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    }

    pub fn wait(&mut self, expected: &[u8], timeout: Duration) -> Result<()> {
        self.wait_any(&[expected], timeout, false).map(|_| ())
    }

    /// Consume through the earliest matching prompt; return its index.
    ///
    /// When `echo` is true, newly received UART bytes are printed live so a
    /// wrong baud rate (garbage) is obvious while waiting.
    pub fn wait_any(&mut self, patterns: &[&[u8]], timeout: Duration, echo: bool) -> Result<usize> {
        if patterns.is_empty()
            || patterns
                .iter()
                .any(|pattern| pattern.is_empty() || pattern.len() > 4096)
        {
            bail!("invalid UART prompt specification");
        }

        let deadline = Instant::now() + timeout;
        let keep = patterns
            .iter()
            .map(|pattern| pattern.len())
            .max()
            .unwrap_or(1)
            - 1;
        let mut input = [0_u8; 1024];

        loop {
            let matched = patterns
                .iter()
                .enumerate()
                .filter_map(|(index, pattern)| {
                    self.pending
                        .windows(pattern.len())
                        .position(|part| part == *pattern)
                        .map(|position| (position, index, pattern.len()))
                })
                .min_by_key(|(position, index, _)| (*position, *index));

            if let Some((position, index, length)) = matched {
                if echo {
                    eprintln!();
                }
                self.pending.drain(..position + length);
                return Ok(index);
            }

            if Instant::now() >= deadline {
                if echo {
                    eprintln!();
                }
                let preview = self.pending_preview();
                bail!(
                    "UART prompt timeout; expected one of {:?}. Recent UART output: {:?}",
                    patterns
                        .iter()
                        .map(|pattern| String::from_utf8_lossy(pattern))
                        .collect::<Vec<_>>(),
                    preview
                );
            }

            if self.pending.len() > 8192 {
                let discard = self.pending.len().saturating_sub(keep);
                self.pending.drain(..discard);
            }

            match self.port.read(&mut input) {
                Ok(0) => std::thread::sleep(Duration::from_millis(5)),
                Ok(count) => {
                    self.log_rx(&input[..count])?;
                    if echo {
                        eprint!("{}", String::from_utf8_lossy(&input[..count]));
                        let _ = std::io::stderr().flush();
                    }
                    self.pending.extend_from_slice(&input[..count]);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }

    /// Collect trailing device RX for a short window (post-flash capture).
    pub fn collect_rx_for(&mut self, duration: Duration) -> Result<()> {
        let deadline = Instant::now() + duration;
        let mut input = [0_u8; 1024];
        while Instant::now() < deadline {
            match self.port.read(&mut input) {
                Ok(0) => std::thread::sleep(Duration::from_millis(20)),
                Ok(count) => {
                    self.log_rx(&input[..count])?;
                    self.pending.extend_from_slice(&input[..count]);
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut
                            | std::io::ErrorKind::WouldBlock
                            | std::io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    /// Send one S-record line at a time with the legacy 10 ms pacing.
    ///
    /// Prints an in-place progress line (percent, bytes, records) while sending.
    /// Device RX is still drained (for prompt integrity) but not written to the
    /// RX log during the bulk upload.
    pub fn send_srec(&mut self, path: &Path, label: &str) -> Result<()> {
        let file = File::open(path)
            .with_context(|| format!("opening S-record {}", path.display()))?;
        let total_bytes = file.metadata()?.len().max(1);
        let mut reader = BufReader::new(file);
        let mut line = Vec::with_capacity(1024);
        let mut sent_bytes: u64 = 0;
        let mut records: u64 = 0;
        let mut last_percent: u8 = 255;
        let started = Instant::now();
        let logging = self.rx_log_enabled;
        self.rx_log_enabled = false;

        let result = (|| {
            loop {
                line.clear();
                let count = {
                    let mut bounded = (&mut reader).take(1025);
                    bounded.read_until(b'\n', &mut line)?
                };

                if count == 0 {
                    break;
                }

                sent_bytes = sent_bytes.saturating_add(count as u64);

                if line.len() > 1024 {
                    eprint!("\n");
                    bail!("S-record line exceeds 1024 bytes in {}", path.display());
                }
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                if !line.starts_with(b"S") {
                    eprint!("\n");
                    bail!("invalid S-record line in {}", path.display());
                }

                self.send(&line)?;
                self.drain_rx()?;
                records += 1;
                std::thread::sleep(Duration::from_millis(10));

                let percent = ((sent_bytes.saturating_mul(100)) / total_bytes).min(100) as u8;
                if percent != last_percent || records == 1 || records % 25 == 0 {
                    last_percent = percent;
                    render_progress(label, percent, sent_bytes, total_bytes, records, started);
                }
            }

            self.drain_rx()?;
            render_progress(label, 100, total_bytes, total_bytes, records, started);
            eprintln!();
            Ok(())
        })();

        self.rx_log_enabled = logging;
        // Bytes drained during the upload sit in `pending` but were not logged;
        // flush them now so post-flash device text (e.g. DRAM errors) is captured.
        if result.is_ok() && logging && !self.pending.is_empty() {
            let pending = self.pending.clone();
            let _ = self.log_rx(&pending);
        }
        result
    }
}

fn render_progress(
    label: &str,
    percent: u8,
    sent_bytes: u64,
    total_bytes: u64,
    records: u64,
    started: Instant,
) {
    const WIDTH: usize = 28;
    let filled = (usize::from(percent) * WIDTH) / 100;
    let bar: String = std::iter::repeat_n('=', filled)
        .chain(std::iter::repeat_n(' ', WIDTH.saturating_sub(filled)))
        .collect();
    let elapsed = started.elapsed().as_secs_f32().max(0.001);
    let rate_kib = (sent_bytes as f32 / 1024.0) / elapsed;

    eprint!(
        "\r{label}: [{bar}] {percent:>3}%  {}/{}  {records} rec  {rate_kib:>5.1} KiB/s",
        format_bytes(sent_bytes),
        format_bytes(total_bytes),
    );
    let _ = std::io::stderr().flush();
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
