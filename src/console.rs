//! Bounded FlashWriter serial channel (adapted from edgecontroller).

use anyhow::{anyhow, bail, Context, Result};
use serialport::SerialPort;
use std::{
    fs::File,
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

pub struct Console {
    port: Box<dyn SerialPort>,
    pending: Vec<u8>,
}

impl Console {
    pub fn open(path: &str, baud: u32) -> Result<Self> {
        Ok(Self {
            port: open_uart(path, baud)?,
            pending: Vec::with_capacity(4096),
        })
    }

    pub fn clear_input(&mut self) -> Result<()> {
        self.port
            .clear(serialport::ClearBuffer::Input)
            .map_err(|error| anyhow!("clearing UART input failed: {error}"))?;
        self.pending.clear();
        Ok(())
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
        self.wait_any(&[expected], timeout).map(|_| ())
    }

    /// Consume through the earliest matching prompt; return its index.
    pub fn wait_any(&mut self, patterns: &[&[u8]], timeout: Duration) -> Result<usize> {
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
                self.pending.drain(..position + length);
                return Ok(index);
            }

            if Instant::now() >= deadline {
                bail!(
                    "UART prompt timeout; expected one of {:?}",
                    patterns
                        .iter()
                        .map(|pattern| String::from_utf8_lossy(pattern))
                        .collect::<Vec<_>>()
                );
            }

            if self.pending.len() > 8192 {
                let discard = self.pending.len().saturating_sub(keep);
                self.pending.drain(..discard);
            }

            match self.port.read(&mut input) {
                Ok(0) => std::thread::sleep(Duration::from_millis(5)),
                Ok(count) => self.pending.extend_from_slice(&input[..count]),
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

    /// Send one S-record line at a time with the legacy 10 ms pacing.
    pub fn send_srec(&mut self, path: &Path) -> Result<()> {
        let mut reader = BufReader::new(File::open(path).with_context(|| {
            format!("opening S-record {}", path.display())
        })?);
        let mut line = Vec::with_capacity(1024);

        loop {
            line.clear();
            let count = {
                let mut bounded = (&mut reader).take(1025);
                bounded.read_until(b'\n', &mut line)?
            };

            if count == 0 {
                break;
            }
            if line.len() > 1024 {
                bail!("S-record line exceeds 1024 bytes in {}", path.display());
            }
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            if !line.starts_with(b"S") {
                bail!("invalid S-record line in {}", path.display());
            }

            self.send(&line)?;
            std::thread::sleep(Duration::from_millis(10));
        }

        Ok(())
    }
}
