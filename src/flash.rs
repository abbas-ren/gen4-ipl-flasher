//! FlashWriter programming steps (from edgecontroller ipl::transfer_image).

use crate::{
    console::{open_rx_log, Console, SrecTransferOptions},
    images::{image_file, Image},
};
use anyhow::{Context, Result};
use std::{path::Path, time::Duration};

fn seconds(value: u64) -> Duration {
    Duration::from_secs(value)
}

/// Wait for a fresh SCIF download prompt after the operator resets the board.
pub fn wait_for_download_prompt(console: &mut Console, timeout_secs: u64) -> Result<()> {
    println!();
    println!("==========================================================================");
    println!("UART is OPEN and listening.");
    println!("NOW reboot / put the board into SCIF download mode with the switches.");
    println!("(Same order as edgecontroller: listen first, then enter download mode.)");
    println!("Waiting for SCIF prompt: please send !");
    println!("Live UART follows (garbage usually means wrong --baud):");
    println!("Device RX is being written to the UART log.");
    println!("==========================================================================");

    console
        .wait_any(
            &[b"please send !", b"please send", b"send !"],
            seconds(timeout_secs),
            true,
        )
        .context("timed out waiting for download-mode prompt")?;
    println!("SCIF download prompt received — starting FlashWriter upload.");
    Ok(())
}

/// Load FlashWriter into RAM over SCIF, then wait for its `>` prompt.
pub fn load_flashwriter(
    console: &mut Console,
    package: &Path,
    flash_writer: &str,
    transfer_options: SrecTransferOptions,
) -> Result<()> {
    let flashwriter = image_file(package, flash_writer)?;
    console.log_marker("FlashWriter .mot send starting (RX log paused during upload)")?;
    println!("Sending FlashWriter ({flash_writer})…");
    console.send_srec(&flashwriter, "FlashWriter", transfer_options)?;

    console.log_marker("FlashWriter .mot send finished — logging device RX again")?;
    println!("FlashWriter .mot transfer finished.");
    println!("Next: waiting for FlashWriter console prompt ('>').");
    println!("Do nothing with switches — FlashWriter should start by itself.");
    println!("Live UART follows:");
    // Give SCIF a moment to jump into FlashWriter after the last S-record.
    std::thread::sleep(Duration::from_millis(500));

    let outcome = console
        .wait_any(
            &[
                b">",
                b"Failed to DRAM initialize",
                b"InitDram error",
                b"Failed to DRAM",
            ],
            seconds(60),
            true,
        )
        .with_context(|| {
            format!(
                "timed out waiting for FlashWriter prompt ('>').\n\
                 Common causes:\n\
                 - wrong baud (try --baud 115200; Gen4 farm default is 921600)\n\
                 - download mode was entered BEFORE the flasher opened UART \
                   (reset into download mode only AFTER 'UART is OPEN')\n\
                 - corrupt/incomplete .mot transfer (USB-UART unreliable at high baud)\n\
                 Recent UART output: {:?}",
                console.pending_preview()
            )
        })?;

    if outcome != 0 {
        anyhow::bail!(
            "FlashWriter started but DRAM init failed (board printed a DRAM error).\n\
             This is not a UART timing bug in the flasher — FlashWriter's built-in \
             LPDDR config does not match this board / power / image.\n\
             Check:\n\
             - Use the exact Gen4 package that works on the farm \
               (ICUMX_Flash_writer_SCIF_DUMMY_CERT_EB203000_V4H.mot for White Hawk)\n\
             - DRAM rails are up before SCIF download mode\n\
             - Board is the matching V4H/White Hawk hardware (wrong SoC/DRAM density \
               needs a rebuilt FlashWriter)\n\
             Recent UART: {:?}",
            console.pending_preview()
        );
    }

    println!("FlashWriter ready (received '>').");
    Ok(())
}

/// Program one image through an already-running FlashWriter session.
pub fn transfer_image(
    console: &mut Console,
    package: &Path,
    image: &Image,
    transfer_options: SrecTransferOptions,
) -> Result<()> {
    let file = image_file(package, &image.name)?;
    console.log_marker(&format!("Programming payload {}", image.name))?;
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

    console.log_marker(&format!(
        "Payload {} S-record send starting (RX log paused during upload)",
        image.name
    ))?;
    console.send_srec(&file, &image.name, transfer_options)?;
    console.log_marker(&format!(
        "Payload {} S-record send finished — logging post-flash device RX",
        image.name
    ))?;

    if image.emmc {
        console
            .wait(b"EM_W Complete!", seconds(60))
            .with_context(|| format!("waiting for eMMC complete for {}", image.name))?;
        console
            .wait(b">", seconds(10))
            .with_context(|| format!("waiting for FlashWriter prompt after {}", image.name))?;
    } else {
        let response = console
            .wait_any(&[b"complete!", b"Clear OK?(y/n)"], seconds(60), false)
            .with_context(|| format!("waiting for program complete for {}", image.name))?;

        if response == 1 {
            console.send(b"y")?;
        }
        // Always synchronize on the command prompt before another image is
        // attempted. In the Clear OK path, the confirmation is sent above.
        console
            .wait(b">", seconds(10))
            .with_context(|| format!("waiting for FlashWriter prompt after {}", image.name))?;
    }

    // Capture any trailing board output after the file finishes.
    console.log_marker(&format!("Trailing RX after {}", image.name))?;
    console.collect_rx_for(Duration::from_secs(2))?;
    Ok(())
}

/// One download-mode session: open UART, wait for a fresh SCIF prompt after
/// the operator resets, load FlashWriter, then program a single image.
pub fn flash_one_image(
    tty: &str,
    baud: u32,
    package: &Path,
    flash_writer: &str,
    image: &Image,
    download_timeout_secs: u64,
    uart_log: &Path,
    transfer_options: SrecTransferOptions,
) -> Result<()> {
    println!("Opening {tty} at {baud} baud (listen BEFORE download-mode reset)…");
    println!("Device RX log: {}", uart_log.display());

    let mut console = Console::open(tty, baud)?;
    console.attach_rx_log(open_rx_log(uart_log)?);
    console.clear_input()?;

    // Log device text from the first please-send wait onward (and after each flash).
    console.log_marker(&format!(
        "Waiting for please send ! before flashing {}",
        image.name
    ))?;
    console.set_rx_logging(true);

    wait_for_download_prompt(&mut console, download_timeout_secs)?;
    load_flashwriter(&mut console, package, flash_writer, transfer_options)?;
    transfer_image(&mut console, package, image, transfer_options)?;

    console.log_marker(&format!("Session finished for {}", image.name))?;
    drop(console);
    println!("Serial port closed. Safe to reboot for the next file when asked.");
    Ok(())
}

pub fn flash_all_images(
    tty: &str,
    baud: u32,
    package: &Path,
    flash_writer: &str,
    images: &[Image],
    download_timeout_secs: u64,
    uart_log: &Path,
    transfer_options: SrecTransferOptions,
) -> Result<()> {
    if images.is_empty() {
        anyhow::bail!("no images selected to flash");
    }

    println!("Opening {tty} at {baud} baud (listen BEFORE download-mode reset)…");
    println!("Device RX log: {}", uart_log.display());

    let mut console = Console::open(tty, baud)?;
    console.attach_rx_log(open_rx_log(uart_log)?);
    console.clear_input()?;
    console.set_rx_logging(true);

    console.log_marker("Starting all-in-one-go flashing session")?;
    wait_for_download_prompt(&mut console, download_timeout_secs)?;

    // FlashWriter is loaded once for this session. The device must return to
    // the FlashWriter command prompt after each payload so the next command
    // can be issued without another SCIF reset.
    load_flashwriter(&mut console, package, flash_writer, transfer_options)?;

    let total = images.len();
    for (index, image) in images.iter().enumerate() {
        let step = index + 1;
        println!("========== [{step}/{total}] {} ==========", image.name);
        console.log_marker(&format!("Starting image [{step}/{total}]: {}", image.name))?;

        transfer_image(&mut console, package, image, transfer_options)
            .with_context(|| format!("flashing image [{step}/{total}] {}", image.name))?;

        console.log_marker(&format!("Completed image [{step}/{total}]: {}", image.name))?;
        println!("OK: {} ({step}/{total})", image.name);

        if step < total {
            // Capture trailing output and allow the command loop to settle before
            // issuing the next FlashWriter command.
            console.collect_rx_for(Duration::from_secs(1))?;
            println!("Next image: {}", images[step].name);
        }
    }

    console.log_marker("All selected images completed in one session")?;
    drop(console);
    println!("Serial port closed.");
    Ok(())
}
