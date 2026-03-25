use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::config::Config;
use crate::curseforge::{CfModLoader, CurseForgeClient};
use crate::modrinth::ModrinthClient;
use crate::scan::ScannedMod;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteSource {
    Modrinth,
    CurseForge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveStatus {
    Pending,
    Resolving,
    UpToDate,
    UpdateAvailable,
    Unknown,
    Error,
}

#[derive(Debug, Clone)]
pub struct ResolvedMod {
    pub scan: ScannedMod,
    pub display_name: String,
    pub local_version: String,
    pub remote_version: Option<String>,
    pub source: Option<RemoteSource>,
    pub status: ResolveStatus,
    pub download_url: Option<String>,
    pub download_filename: Option<String>,
    pub detail: Option<String>,
    pub project_label: Option<String>,
}

pub fn is_newer_version(local: &str, remote: &str) -> bool {
    let l = local.trim();
    let r = remote.trim();
    if l == r {
        return false;
    }
    match (semver::Version::parse(l), semver::Version::parse(r)) {
        (Ok(a), Ok(b)) => b > a,
        _ => true,
    }
}

fn primary_modrinth_file(version: &crate::modrinth::ModrinthVersion) -> Option<(String, String)> {
    let f = version
        .files
        .iter()
        .find(|f| f.primary == Some(true))
        .or_else(|| version.files.first())?;
    Some((f.url.clone(), f.filename.clone()))
}

pub async fn resolve_all(
    config: Arc<Config>,
    scans: Vec<ScannedMod>,
    modrinth: Arc<ModrinthClient>,
    curse: Option<Arc<CurseForgeClient>>,
) -> Vec<ResolvedMod> {
    let sem = Arc::new(Semaphore::new(config.concurrency.max(1)));
    let loaders = config.normalized_loaders();
    let game_versions = vec![config.minecraft_version().to_string()];
    let mc = config.minecraft_version().to_string();
    let mut tasks = Vec::new();
    for scan in scans {
        let config = Arc::clone(&config);
        let mr = Arc::clone(&modrinth);
        let curse = curse.as_ref().map(Arc::clone);
        let sem = Arc::clone(&sem);
        let loaders = loaders.clone();
        let game_versions = game_versions.clone();
        let mc = mc.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.ok();
            resolve_one(
                &config,
                scan,
                &mr,
                curse.as_ref(),
                &loaders,
                &game_versions,
                &mc,
            )
            .await
        }));
    }
    let mut out = Vec::new();
    for t in tasks {
        if let Ok(r) = t.await {
            out.push(r);
        }
    }
    out.sort_by(|a, b| a.scan.file_name.cmp(&b.scan.file_name));
    out
}

async fn resolve_one(
    config: &Config,
    scan: ScannedMod,
    modrinth: &ModrinthClient,
    curse: Option<&Arc<CurseForgeClient>>,
    loaders: &[String],
    game_versions: &[String],
    mc: &str,
) -> ResolvedMod {
    let (display_name, local_version) = match &scan.metadata {
        Some(m) => (m.display_name.clone(), m.version.clone()),
        None => (scan.file_name.clone(), "?".to_string()),
    };

    if let Ok(Some(ver)) = modrinth
        .version_from_hash_update(&scan.sha512_hex, loaders, game_versions)
        .await
    {
        let remote_v = ver.version_number.clone();
        let project_label = modrinth
            .get_project(&ver.project_id)
            .await
            .ok()
            .map(|p| format!("{} ({})", p.title, p.slug));
        let (download_url, download_filename) = match primary_modrinth_file(&ver) {
            Some((u, f)) => (Some(u), Some(f)),
            None => (None, None),
        };
        let status = if local_version == "?" || is_newer_version(&local_version, &remote_v) {
            ResolveStatus::UpdateAvailable
        } else {
            ResolveStatus::UpToDate
        };
        return ResolvedMod {
            scan,
            display_name,
            local_version,
            remote_version: Some(remote_v),
            source: Some(RemoteSource::Modrinth),
            status,
            download_url,
            download_filename,
            detail: None,
            project_label,
        };
    }

    if let Some(cf) = curse {
        let fp = match crate::curseforge::fingerprint_file(&scan.path) {
            Ok(f) => f,
            Err(e) => {
                return error_row(
                    scan,
                    display_name,
                    local_version,
                    format!("fingerprint: {}", e),
                );
            }
        };
        if let Ok(matches) = cf.fingerprint_matches(&[fp]).await {
            if let Some(m) = matches.first() {
                let loader = config
                    .normalized_loaders()
                    .first()
                    .and_then(|s| CfModLoader::from_loader_str(s));
                let latest = cf
                    .latest_file_for_game(m.mod_id, mc, loader)
                    .await
                    .ok()
                    .flatten();
                if let Some(lf) = latest {
                    let remote_v = guess_version_from_filename(&lf.file_name)
                        .unwrap_or_else(|| lf.file_name.clone());
                    let mod_title = cf.get_mod_description(m.mod_id).await.ok();
                    let status = if lf.file_id == m.file_id {
                        ResolveStatus::UpToDate
                    } else {
                        ResolveStatus::UpdateAvailable
                    };
                    return ResolvedMod {
                        scan,
                        display_name,
                        local_version,
                        remote_version: Some(remote_v),
                        source: Some(RemoteSource::CurseForge),
                        status,
                        download_url: lf.download_url,
                        download_filename: Some(lf.file_name),
                        detail: None,
                        project_label: mod_title,
                    };
                }
                return ResolvedMod {
                    scan,
                    display_name,
                    local_version,
                    remote_version: None,
                    source: Some(RemoteSource::CurseForge),
                    status: ResolveStatus::Unknown,
                    download_url: None,
                    download_filename: None,
                    detail: Some("No file for target MC/loader on CurseForge".into()),
                    project_label: None,
                };
            }
        }

        let search_q = display_name.clone();
        if let Ok(hits) = cf.search_mods(&search_q, 5).await {
            if let Some(h) = hits.first() {
                let loader = config
                    .normalized_loaders()
                    .first()
                    .and_then(|s| CfModLoader::from_loader_str(s));
                if let Ok(Some(lf)) = cf.latest_file_for_game(h.id, mc, loader).await {
                    let remote_v = guess_version_from_filename(&lf.file_name)
                        .unwrap_or_else(|| lf.file_name.clone());
                    let status = if lf.file_name == scan.file_name {
                        ResolveStatus::UpToDate
                    } else {
                        ResolveStatus::UpdateAvailable
                    };
                    return ResolvedMod {
                        scan,
                        display_name,
                        local_version,
                        remote_version: Some(remote_v),
                        source: Some(RemoteSource::CurseForge),
                        status,
                        download_url: lf.download_url,
                        download_filename: Some(lf.file_name),
                        detail: Some("matched via search (verify)".into()),
                        project_label: Some(format!("{} ({})", h.name, h.slug)),
                    };
                }
            }
        }
    }

    ResolvedMod {
        scan,
        display_name,
        local_version,
        remote_version: None,
        source: None,
        status: ResolveStatus::Unknown,
        download_url: None,
        download_filename: None,
        detail: Some("Not on Modrinth; CurseForge key missing or no match".into()),
        project_label: None,
    }
}

fn error_row(
    scan: ScannedMod,
    display_name: String,
    local_version: String,
    msg: String,
) -> ResolvedMod {
    ResolvedMod {
        scan,
        display_name,
        local_version,
        remote_version: None,
        source: None,
        status: ResolveStatus::Error,
        download_url: None,
        download_filename: None,
        detail: Some(msg),
        project_label: None,
    }
}

fn guess_version_from_filename(name: &str) -> Option<String> {
    let base = name.strip_suffix(".jar").unwrap_or(name);
    let parts: Vec<&str> = base.split('-').collect();
    if parts.len() >= 2 {
        let last = parts[parts.len() - 1];
        if last
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
        {
            return Some(last.to_string());
        }
    }
    None
}
