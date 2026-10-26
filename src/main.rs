//! Manual Gen4 IPL flasher.
//!
//! Uses the same FlashWriter UART protocol as edgecontroller, but leaves
//! download-mode entry and board reboots to the operator (physical switches).
//! After each payload image is programmed, the tool asks you to reboot into
//! download mode again before continuing with the next file in order.

mod console;
mod flash;
mod images;

use anyhow::{bail, Context, Result};
use clap::Parser;
use images::{package_dir, preflight, IMAGES};
use std::{
    io::{self, Write},
    path::PathBuf,
};

#[derive(Debug, Parser)]
#[command(
    name = "gen4-ipl-flasher",
    about = "Flash Gen4 IPL images over UART with manual download mode / reboot",
    long_about = "FlashWriter and payload S-records are taken from an images folder \
(same layout as edgecontroller Gen4 packages). Put the board into SCIF download \
mode with switches before each image. After each successful program, reboot into \
download mode again when prompted."
)]
struct Cli {
    /// Serial device, e.g. /dev/ttyUSB0
    #[arg(short, long, value_name = "TTY")]
    tty: String,

    /// Directory containing FlashWriter .mot and the Gen4 .srec images
    #[arg(short, long, value_name = "DIR")]
    images: PathBuf,

    /// UART baud rate (FlashWriter SCIF default)
    #[arg(long, default_value_t = 921_600)]
    baud: u32,

    /// Seconds to wait for the SCIF "please send !" prompt after Enter
    #[arg(long, default_value_t = 120)]
    download_timeout: u64,

    /// 0-based index into the Gen4 image list to start from (resume)
    #[arg(long, default_value_t = 0)]
    start_from: usize,
}

fn prompt_enter(message: &str) -> Result<()> {
    print!("{message}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .context("reading operator confirmation")?;
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.start_from >= IMAGES.len() {
        bail!(
            "--start-from {} is out of range (0..{})",
            cli.start_from,
            IMAGES.len() - 1
        );
    }

    let package = package_dir(&cli.images)?;
    println!("Preflight check under {}…", package.display());
    preflight(&package)?;
    println!("All required FlashWriter / S-record files are present.\n");

    println!("Image order:");
    for (index, image) in IMAGES.iter().enumerate() {
        let marker = if index < cli.start_from { "skip" } else { "todo" };
        println!("  [{marker}] {:>2}. {}", index + 1, image.name);
    }
    println!();
    println!("TTY={}  baud={}", cli.tty, cli.baud);
    println!("Download mode and power/reboot are manual (switches).\n");

    let total = IMAGES.len();
    for (index, image) in IMAGES.iter().enumerate().skip(cli.start_from) {
        let step = index + 1;
        println!("========== [{step}/{total}] {} ==========", image.name);

        if index == cli.start_from {
            prompt_enter(
                "Put the board into SCIF download mode with switches, then press Enter… ",
            )?;
        } else {
            prompt_enter(&format!(
                "Reboot the board into SCIF download mode for the next file ({}), then press Enter… ",
                image.name
            ))?;
        }

        flash::flash_one_image(
            &cli.tty,
            cli.baud,
            &package,
            image,
            cli.download_timeout,
        )
        .with_context(|| format!("flashing {}", image.name))?;

        println!("OK: {} ({step}/{total})\n", image.name);
    }

    println!("All Gen4 IPL images flashed successfully.");
    println!("You can leave download mode / reboot to normal boot with switches.");
    Ok(())
}
