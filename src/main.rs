//! Manual Gen4 IPL flasher.
//!
//! Uses the same FlashWriter UART protocol as edgecontroller, but leaves
//! download-mode entry and board reboots to the operator (physical switches).
//!
//! Critical timing (same as edgecontroller): the flasher opens the UART and
//! starts listening FIRST, then you reset into SCIF download mode so the
//! fresh "please send !" prompt is captured. Entering download mode before
//! the port is open often yields a successful-looking .mot send with no '>'.
//!
//! Package layout is described by `ipl.json` in the images directory.

mod console;
mod flash;
mod images;

use anyhow::{bail, Context, Result};
use clap::Parser;
use images::{package_dir, preflight};
use std::{
    io::{self, Write},
    path::PathBuf,
};

#[derive(Debug, Parser)]
#[command(
    name = "gen4-ipl-flasher",
    about = "Flash Gen4 IPL images over UART with manual download mode / reboot",
    long_about = "FlashWriter and payload S-records come from an images folder \
with ipl.json (same layout as edgecontroller Gen4 packages). For each file the \
flasher opens UART first; you then reboot into SCIF download mode with switches."
)]
struct Cli {
    /// Serial device, e.g. /dev/ttyUSB0
    #[arg(short, long, value_name = "TTY")]
    tty: String,

    /// Directory containing ipl.json, FlashWriter .mot, and Gen4 .srec images
    #[arg(short, long, value_name = "DIR")]
    images: PathBuf,

    /// UART baud rate (edgecontroller Gen4 uses 921600; try 115200 if UART looks like garbage)
    #[arg(long, default_value_t = 921_600)]
    baud: u32,

    /// Seconds to wait for the SCIF "please send !" prompt after UART opens
    #[arg(long, default_value_t = 120)]
    download_timeout: u64,

    /// 0-based index into the Gen4 image list to start from (resume)
    #[arg(long, default_value_t = 0)]
    start_from: usize,

    /// Flash all selected images in one UART/download-mode session.
    ///
    /// Opens UART once, waits for one SCIF download prompt, loads FlashWriter
    /// once, then programs each selected image without asking for another reset.
    /// Use only if the board's FlashWriter returns to its command prompt after
    /// each image is programmed.
    #[arg(long, default_value_t = false)]
    all: bool,

    /// Append device RX here (from first please-send wait; paused during S-record uploads)
    #[arg(long, default_value = "gen4-ipl-uart.log", value_name = "FILE")]
    uart_log: PathBuf,
}

/// Confirm the operator is ready. Download-mode reset happens AFTER UART opens.
fn confirm_ready_to_listen(
    image_name: &str,
    step: usize,
    total: usize,
    is_first: bool,
) -> Result<()> {
    println!();
    println!("--------------------------------------------------------------------------");
    if is_first {
        println!("READY FOR FILE [{step}/{total}]: {image_name}");
    } else {
        println!("NEXT FILE [{step}/{total}]: {image_name}");
        println!("Prepare to reboot into SCIF download mode again.");
    }
    println!();
    println!("IMPORTANT — do NOT enter download mode yet.");
    println!("1. Press Enter here first (flasher will open UART and listen).");
    println!("2. When it says \"UART is OPEN\", THEN reset into download mode.");
    println!("   That is how edgecontroller catches a fresh \"please send !\".");
    println!("--------------------------------------------------------------------------");
    print!("Press Enter to open UART and start listening… ");
    io::stdout().flush()?;

    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .context("reading operator confirmation")?;

    println!("Opening session for {image_name}…");
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let package = package_dir(&cli.images)?;
    println!("Preflight check under {}…", package.display());
    let config = preflight(&package)?;
    println!(
        "FlashWriter={}  images={}",
        config.flash_writer,
        config.images.len()
    );

    if cli.start_from >= config.images.len() {
        bail!(
            "--start-from {} is out of range (0..{})",
            cli.start_from,
            config.images.len().saturating_sub(1)
        );
    }

    println!("Image order:");
    for (index, image) in config.images.iter().enumerate() {
        let marker = if index < cli.start_from {
            "skip"
        } else {
            "todo"
        };
        println!("  [{marker}] {:>2}. {}", index + 1, image.name);
    }
    println!();
    println!("TTY={}  baud={}", cli.tty, cli.baud);
    println!("UART RX log={}", cli.uart_log.display());
    println!("Listen first, then reset into download mode (matches edgecontroller).\n");

    let total = config.images.len();
    let images: Vec<_> = config
        .images
        .iter()
        .enumerate()
        .skip(cli.start_from)
        .collect();

    if cli.all {
        let selected_images: Vec<_> = images.iter().map(|(_, image)| (**image).clone()).collect();
        println!("ALL-IN-ONE-GO mode: one UART session and one download-mode reset.");
        println!("FlashWriter will be loaded once; each selected payload will then be programmed.");
        println!(
            "Verify the board's FlashWriter supports programming multiple images in one session."
        );

        if let Some((index, image)) = images.first() {
            confirm_ready_to_listen(&image.name, index + 1, total, true)?;
        }

        flash::flash_all_images(
            &cli.tty,
            cli.baud,
            &package,
            &config.flash_writer,
            &selected_images,
            cli.download_timeout,
            &cli.uart_log,
        )
        .context("flashing all selected images in one UART session")?;
    } else {
        for (position, (index, image)) in images.iter().enumerate() {
            let step = index + 1;
            let is_first = position == 0;

            confirm_ready_to_listen(&image.name, step, total, is_first)?;

            println!("========== [{step}/{total}] {} ==========", image.name);
            flash::flash_one_image(
                &cli.tty,
                cli.baud,
                &package,
                &config.flash_writer,
                image,
                cli.download_timeout,
                &cli.uart_log,
            )
            .with_context(|| format!("flashing {}", image.name))?;

            println!("OK: {} ({step}/{total})", image.name);

            if position + 1 < images.len() {
                let next = &images[position + 1].1.name;
                println!();
                println!(
                    "Finished {current}. Next up is {next}.",
                    current = image.name
                );
            }
        }
    }

    println!();
    println!("All selected Gen4 IPL images flashed successfully.");
    println!("You can leave download mode / reboot to normal boot with switches.");
    Ok(())
}
