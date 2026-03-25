use std::path::{Path, PathBuf};

use reqwest::Client;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::config::Config;
use crate::resolve::ResolvedMod;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("no download URL")]
    NoUrl,
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub async fn backup_file(src: &Path, backup_root: &Path) -> std::io::Result<PathBuf> {
    let name = src
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mod.jar".into());
    fs::create_dir_all(backup_root).await?;
    let dest = backup_root.join(name);
    fs::copy(src, &dest).await?;
    Ok(dest)
}

pub async fn download_mod_update(
    client: &Client,
    config: &Config,
    row: &ResolvedMod,
) -> Result<PathBuf, DownloadError> {
    let url = row.download_url.as_ref().ok_or(DownloadError::NoUrl)?;
    if url.is_empty() {
        return Err(DownloadError::NoUrl);
    }
    let dest_path = row.scan.path.clone();
    if config.download.dry_run {
        return Ok(dest_path);
    }
    let name = dest_path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "mod.jar".into());
    let part_path = dest_path.with_file_name(format!("{name}.part"));

    if config.download.backup {
        let day = chrono_date_folder();
        let backup_dir = config.resolved_backup_dir().join(day);
        backup_file(&dest_path, &backup_dir).await?;
    }

    let mut resp = client.get(url).send().await?.error_for_status()?;
    let mut file = fs::File::create(&part_path).await?;
    while let Some(chunk) = resp.chunk().await? {
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    fs::rename(&part_path, &dest_path).await?;
    Ok(dest_path)
}

fn chrono_date_folder() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}
