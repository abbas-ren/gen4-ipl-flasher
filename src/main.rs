//! Manual Gen4 IPL flasher.
//!
//! Uses the same FlashWriter UART protocol as edgecontroller, but leaves
//! download-mode entry and board reboots to the operator (physical switches).
//!
//! Important: after each image, the tool blocks on operator confirmation and
//! does not open the UART or wait for any device prompts until you confirm
//! the board has been rebooted into download mode for the next file.

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
download mode again when prompted. The flasher will not listen for UART prompts \
until you confirm that reboot."
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

    /// Seconds to wait for the SCIF "please send !" prompt after confirmation
    #[arg(long, default_value_t = 120)]
    download_timeout: u64,

    /// 0-based index into the Gen4 image list to start from (resume)
    #[arg(long, default_value_t = 0)]
    start_from: usize,
}

/// Block until the operator confirms. No UART I/O happens before this returns.
fn confirm_download_mode_ready(image_name: &str, step: usize, total: usize, is_first: bool) -> Result<()> {
    println!();
    println!("--------------------------------------------------------------------------");
    if is_first {
        println!("READY FOR FILE [{step}/{total}]: {image_name}");
        println!("1. Put the board into SCIF download mode using the switches.");
        println!("2. Press Enter ONLY after download mode is active.");
    } else {
        println!("NEXT FILE [{step}/{total}]: {image_name}");
        println!("1. Reboot the board into SCIF download mode using the switches.");
        println!("2. Press Enter ONLY after that reboot/download-mode is complete.");
    }
    println!("The flasher will NOT open the serial port or wait for device prompts");
    println!("until you press Enter here.");
    println!("--------------------------------------------------------------------------");
    print!("Press Enter when the device is ready in download mode… ");
    io::stdout().flush()?;

    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .context("reading operator confirmation")?;

    println!("Confirmed. Starting flash session for {image_name}…");
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
    println!("Download mode and power/reboot are manual (switches).");
    println!("UART waits start only after each of your confirmations.\n");

    let total = IMAGES.len();
    let images: Vec<_> = IMAGES.iter().enumerate().skip(cli.start_from).collect();

    for (position, (index, image)) in images.iter().enumerate() {
        let step = index + 1;
        let is_first = position == 0;

        // Gate: no UART activity for this file until the operator confirms.
        confirm_download_mode_ready(image.name, step, total, is_first)?;

        println!("========== [{step}/{total}] {} ==========", image.name);
        flash::flash_one_image(
            &cli.tty,
            cli.baud,
            &package,
            image,
            cli.download_timeout,
        )
        .with_context(|| format!("flashing {}", image.name))?;

        println!("OK: {} ({step}/{total})", image.name);

        if position + 1 < images.len() {
            let next = images[position + 1].1.name;
            println!();
            println!(
                "Finished {current}. Next up is {next}.",
                current = image.name
            );
            println!(
                "Do not continue until you have rebooted the device into download mode for {next}."
            );
        }
    }

    println!();
    println!("All Gen4 IPL images flashed successfully.");
    println!("You can leave download mode / reboot to normal boot with switches.");
    Ok(())
}
