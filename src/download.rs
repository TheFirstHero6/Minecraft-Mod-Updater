use std::path::{Path, PathBuf};

use reqwest::Client;
use tokio::fs;
use tokio::io::AsyncWriteExt;

use crate::config::Config;
use crate::resolve::ResolvedMod;
use crate::scan::hash_file_sha512;

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("no download URL")]
    NoUrl,
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("verification failed: {0}")]
    Verify(String),
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

    let backup_dest = if config.download.backup {
        let day = chrono_date_folder();
        let backup_dir = config.resolved_backup_dir().join(day);
        Some(backup_file(&dest_path, &backup_dir).await?)
    } else {
        None
    };

    let mut resp = client.get(url).send().await?.error_for_status()?;
    let mut file = fs::File::create(&part_path).await?;
    while let Some(chunk) = resp.chunk().await? {
        file.write_all(&chunk).await?;
    }
    file.flush().await?;
    drop(file);
    fs::rename(&part_path, &dest_path).await?;

    if let Some(expected_sha512) = row.remote_file_sha512.as_deref() {
        let actual_sha512 =
            hash_file_sha512(&dest_path).map_err(|e| DownloadError::Verify(e.to_string()))?;
        if !actual_sha512.eq_ignore_ascii_case(expected_sha512) {
            rollback_download(&dest_path, backup_dest.as_deref()).await?;
            return Err(DownloadError::Verify(format!(
                "downloaded file hash mismatch (expected {}, got {})",
                short_hash(expected_sha512),
                short_hash(&actual_sha512)
            )));
        }
    }

    if config.download.verify_after_download {
        if let Err(e) = crate::verify::verify_update_jar(
            &dest_path,
            config.minecraft_version(),
            &config.normalized_loaders(),
        ) {
            rollback_download(&dest_path, backup_dest.as_deref()).await?;
            return Err(DownloadError::Verify(e));
        }
    }

    Ok(dest_path)
}

fn chrono_date_folder() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

async fn rollback_download(dest_path: &Path, backup_path: Option<&Path>) -> Result<(), DownloadError> {
    if let Some(bp) = backup_path {
        fs::copy(bp, dest_path).await?;
    } else {
        let _ = fs::remove_file(dest_path).await;
    }
    Ok(())
}

fn short_hash(hash: &str) -> &str {
    hash.get(..8).unwrap_or(hash)
}
