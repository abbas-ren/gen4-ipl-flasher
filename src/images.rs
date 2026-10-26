//! Gen4 FlashWriter image list (from edgecontroller).

use anyhow::{bail, Context, Result};
use std::{fs, path::{Path, PathBuf}};

pub const FLASHWRITER: &str = "ICUMX_Flash_writer_SCIF_DUMMY_CERT_EB203000_V4H.mot";
pub const MAX_IMAGE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub struct Image {
    pub name: &'static str,
    pub save: &'static str,
    pub address: &'static str,
    pub emmc: bool,
}

/// Programming order used by edgecontroller Gen3/Gen4 FlashWriter.
pub const IMAGES: [Image; 8] = [
    Image {
        name: "bootparam_sa0.srec",
        save: "0",
        address: "EB200000",
        emmc: false,
    },
    Image {
        name: "icumx_loader.srec",
        save: "40000",
        address: "EB210000",
        emmc: false,
    },
    Image {
        name: "cert_header_sa9.srec",
        save: "240000",
        address: "EB230000",
        emmc: false,
    },
    Image {
        name: "dummy_fw.srec",
        save: "280000",
        address: "EB240000",
        emmc: false,
    },
    Image {
        name: "cr52_loader.srec",
        save: "480000",
        address: "E6300000",
        emmc: false,
    },
    Image {
        name: "dummy_rtos.srec",
        save: "2800",
        address: "E2100000",
        emmc: true,
    },
    Image {
        name: "bl31-whitehawk.srec",
        save: "a000",
        address: "46400000",
        emmc: true,
    },
    Image {
        name: "u-boot-elf-whitehawk.srec",
        save: "ac00",
        address: "50000000",
        emmc: true,
    },
];

pub fn image_file(package: &Path, name: &str) -> Result<PathBuf> {
    let path = fs::canonicalize(package.join(name))
        .with_context(|| format!("missing firmware file {name} under {}", package.display()))?;
    let metadata = fs::metadata(&path)?;

    if !path.starts_with(package)
        || !metadata.is_file()
        || metadata.len() == 0
        || metadata.len() > MAX_IMAGE_BYTES
    {
        bail!("invalid or oversized firmware image: {name}");
    }

    Ok(path)
}

pub fn preflight(package: &Path) -> Result<()> {
    let package = fs::canonicalize(package)
        .with_context(|| format!("images folder {}", package.display()))?;
    if !package.is_dir() {
        bail!("{} is not a directory", package.display());
    }

    image_file(&package, FLASHWRITER)?;
    for image in &IMAGES {
        image_file(&package, image.name)?;
    }

    Ok(())
}

pub fn package_dir(path: &Path) -> Result<PathBuf> {
    let package = fs::canonicalize(path)
        .with_context(|| format!("images folder {}", path.display()))?;
    if !package.is_dir() {
        bail!("{} is not a directory", package.display());
    }
    Ok(package)
}
