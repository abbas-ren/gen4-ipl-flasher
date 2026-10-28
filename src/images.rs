//! Gen4 FlashWriter package layout from `ipl.json`.

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub const IPL_CONFIG_FILE: &str = "ipl.json";
pub const MAX_IMAGE_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct Image {
    pub name: String,
    pub save: String,
    pub address: String,
    pub emmc: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IplPackageConfig {
    pub flash_writer: String,
    pub images: Vec<Image>,
}

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

pub fn load_package_config(package: &Path) -> Result<IplPackageConfig> {
    let path = package.join(IPL_CONFIG_FILE);
    let text = fs::read_to_string(&path)
        .with_context(|| format!("reading {} under {}", IPL_CONFIG_FILE, package.display()))?;
    let config: IplPackageConfig = serde_json::from_str(&text)
        .with_context(|| format!("invalid {IPL_CONFIG_FILE}"))?;

    if config.flash_writer.trim().is_empty() {
        bail!("{IPL_CONFIG_FILE}: flashWriter must be non-empty");
    }
    if config.flash_writer.contains('/') || config.flash_writer.contains('\\') {
        bail!("{IPL_CONFIG_FILE}: flashWriter must be a bare file name");
    }
    if config.images.is_empty() {
        bail!("{IPL_CONFIG_FILE}: images must not be empty");
    }
    for image in &config.images {
        if image.name.trim().is_empty()
            || image.save.trim().is_empty()
            || image.address.trim().is_empty()
        {
            bail!("{IPL_CONFIG_FILE}: each image needs name, save, and address");
        }
        if image.name.contains('/') || image.name.contains('\\') {
            bail!(
                "{IPL_CONFIG_FILE}: image name must be a bare file name: {}",
                image.name
            );
        }
    }

    Ok(config)
}

pub fn preflight(package: &Path) -> Result<IplPackageConfig> {
    let package = package_dir(package)?;
    let config = load_package_config(&package)?;
    image_file(&package, &config.flash_writer)?;
    for image in &config.images {
        image_file(&package, &image.name)?;
    }
    Ok(config)
}

pub fn package_dir(path: &Path) -> Result<PathBuf> {
    let package = fs::canonicalize(path)
        .with_context(|| format!("images folder {}", path.display()))?;
    if !package.is_dir() {
        bail!("{} is not a directory", package.display());
    }
    Ok(package)
}
