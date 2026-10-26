//! FlashWriter programming steps (from edgecontroller ipl::transfer_image).

use crate::{
    console::Console,
    images::{image_file, Image, FLASHWRITER},
};
use anyhow::{Context, Result};
use std::{
    path::Path,
    time::Duration,
};

fn seconds(value: u64) -> Duration {
    Duration::from_secs(value)
}

/// Wait for SCIF download prompt after the operator enables download mode.
pub fn wait_for_download_prompt(console: &mut Console, timeout_secs: u64) -> Result<()> {
    println!("Waiting for SCIF download prompt (please send !)…");
    console
        .wait_any(
            &[b"please send !", b"please send", b"send !"],
            seconds(timeout_secs),
        )
        .context("timed out waiting for download-mode prompt")?;
    Ok(())
}

/// Load FlashWriter into RAM over SCIF, then wait for its `>` prompt.
pub fn load_flashwriter(console: &mut Console, package: &Path) -> Result<()> {
    let flashwriter = image_file(package, FLASHWRITER)?;
    println!("Sending FlashWriter ({FLASHWRITER})…");
    console.send_srec(&flashwriter)?;
    console
        .wait(b">", seconds(30))
        .context("timed out waiting for FlashWriter prompt")?;
    println!("FlashWriter ready.");
    Ok(())
}

/// Program one image through an already-running FlashWriter session.
pub fn transfer_image(console: &mut Console, package: &Path, image: &Image) -> Result<()> {
    let file = image_file(package, image.name)?;
    println!(
        "Programming {} (save={}, address={}, emmc={})",
        image.name, image.save, image.address, image.emmc
    );

    if image.emmc {
        console.command("em_w\r\n")?;
        console.wait(b"(Push Y key)", seconds(10))?;
        console.send(b"y")?;

        console.wait(b"Select area(0-2)>", seconds(10))?;
        console.command("1\r\n")?;

        console.wait(b"Please Input Start Address in sector :", seconds(10))?;
        console.command(&format!("{}\r\n", image.save))?;

        console.wait(b"Please Input Program Start Address :", seconds(10))?;
        console.command(&format!("{}\r\n", image.address))?;
    } else {
        console.command("xls2\r\n")?;
        console.wait(b"Select (1-3)>", seconds(10))?;
        console.command("1\r\n")?;

        console.wait(b"(Push Y key)", seconds(10))?;
        console.send(b"y")?;

        console.wait(b"(Push Y key)", seconds(10))?;
        console.send(b"y")?;

        console.wait(b"Please Input : H", seconds(10))?;
        console.command(&format!("{}\r\n", image.address))?;

        console.wait(b"Please Input : H", seconds(10))?;
        console.command(&format!("{}\r\n", image.save))?;
    }

    console.wait(b"please send ! (Motorola S-record)", seconds(15))?;
    std::thread::sleep(Duration::from_millis(400));

    console.send_srec(&file)?;

    if image.emmc {
        console
            .wait(b"EM_W Complete!", seconds(60))
            .with_context(|| format!("waiting for eMMC complete for {}", image.name))?;
    } else {
        let response = console
            .wait_any(&[b"complete!", b"Clear OK?(y/n)"], seconds(60))
            .with_context(|| format!("waiting for program complete for {}", image.name))?;

        if response == 1 {
            console.send(b"y")?;
            console.wait(b">", seconds(10))?;
        }
    }

    Ok(())
}

/// One download-mode session: load FlashWriter, then program a single image.
pub fn flash_one_image(
    tty: &str,
    baud: u32,
    package: &Path,
    image: &Image,
    download_timeout_secs: u64,
) -> Result<()> {
    let mut console = Console::open(tty, baud)?;
    console.clear_input()?;
    wait_for_download_prompt(&mut console, download_timeout_secs)?;
    load_flashwriter(&mut console, package)?;
    transfer_image(&mut console, package, image)?;
    // Drop UART before the operator reboots for the next image.
    drop(console);
    Ok(())
}
