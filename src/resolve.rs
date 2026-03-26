use std::sync::Arc;
use std::{cmp::Ordering, collections::BTreeSet};

use tokio::sync::Semaphore;

use crate::config::Config;
use crate::curseforge::{CfModLoader, CurseForgeClient, CfFileSummary};
use crate::mc_version::filename_declares_mc;
use crate::modrinth::{ModrinthClient, ModrinthFile, ModrinthVersion};
use crate::scan::{hash_file_sha1, ScannedMod};

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
    pub remote_file_sha512: Option<String>,
    pub identity_match: Option<bool>,
}

pub fn is_newer_version(local: &str, remote: &str) -> bool {
    let l = local.trim();
    let r = remote.trim();
    if l == r {
        return false;
    }
    if let (Ok(a), Ok(b)) = (semver::Version::parse(l), semver::Version::parse(r)) {
        return b > a;
    }
    if let (Ok(a), Ok(b)) = (
        semver::Version::parse(l.split('+').next().unwrap_or(l)),
        semver::Version::parse(r.split('+').next().unwrap_or(r)),
    ) {
        return b > a;
    }
    false
}

fn modrinth_response_matches_request(ver: &ModrinthVersion, mc: &str, loaders: &[String]) -> bool {
    if !ver.game_versions.is_empty()
        && !ver
            .game_versions
            .iter()
            .any(|g| g.trim().eq_ignore_ascii_case(mc.trim()))
    {
        return false;
    }
    if !ver.loaders.is_empty()
        && !loaders
            .iter()
            .any(|want| ver.loaders.iter().any(|v| v.eq_ignore_ascii_case(want)))
    {
        return false;
    }
    true
}

fn loader_matches_file(f: &ModrinthFile, loaders: &[String]) -> bool {
    if f.loaders.is_empty() {
        return true;
    }
    loaders
        .iter()
        .any(|want| f.loaders.iter().any(|v| v.eq_ignore_ascii_case(want)))
}

fn pick_modrinth_file<'a>(
    ver: &'a ModrinthVersion,
    loaders: &[String],
    mc: &str,
) -> Option<&'a ModrinthFile> {
    let jars: Vec<&ModrinthFile> = ver
        .files
        .iter()
        .filter(|f| {
            let n = f.filename.to_ascii_lowercase();
            n.ends_with(".jar")
                && !n.contains("sources")
                && !n.contains("javadoc")
                && !n.contains("-dev.")
        })
        .collect();
    if jars.is_empty() {
        return None;
    }
    let mut matched: Vec<&ModrinthFile> = jars
        .iter()
        .copied()
        .filter(|f| loader_matches_file(f, loaders))
        .collect();
    if matched.is_empty() {
        matched = jars;
    }
    let mc_pref: Vec<&ModrinthFile> = matched
        .iter()
        .copied()
        .filter(|f| filename_declares_mc(&f.filename, mc))
        .collect();
    let pool: Vec<&ModrinthFile> = if !mc_pref.is_empty() {
        mc_pref
    } else {
        matched
    };
    let chosen = pool
        .iter()
        .find(|f| f.primary == Some(true))
        .copied()
        .or_else(|| pool.first().copied())?;
    Some(chosen)
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
    out.sort_by(|a, b| {
        a.display_name
            .to_ascii_lowercase()
            .cmp(&b.display_name.to_ascii_lowercase())
            .then_with(|| a.scan.file_name.cmp(&b.scan.file_name))
    });
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

    let modrinth_detail = match modrinth
        .version_from_hash_update(&scan.sha512_hex, loaders, game_versions)
        .await
    {
        Ok(Some(ver)) => {
            if modrinth_response_matches_request(&ver, mc, loaders) {
                let remote_v = ver.version_number.clone();
                let project_label = modrinth
                    .get_project(&ver.project_id)
                    .await
                    .ok()
                    .map(|p| format!("{} ({})", p.title, p.slug));
                let chosen = pick_modrinth_file(&ver, loaders, mc);
                let download_url = chosen.map(|f| f.url.clone());
                let download_filename = chosen.map(|f| f.filename.clone());
                let remote_file_sha512 = chosen.and_then(|f| f.hashes.sha512.clone());
                let identity_match = remote_file_sha512
                    .as_deref()
                    .map(|sha| sha.eq_ignore_ascii_case(&scan.sha512_hex));
                let (status, detail) = if download_url.is_none() {
                    (
                        ResolveStatus::Unknown,
                        Some("Modrinth: no matching .jar for requested loader/MC in this version".into()),
                    )
                } else if identity_match == Some(true) {
                    (ResolveStatus::UpToDate, None)
                } else if identity_match == Some(false) {
                    (ResolveStatus::UpdateAvailable, None)
                } else {
                    (
                        ResolveStatus::Unknown,
                        Some("Modrinth: selected file is missing sha512 in API response".into()),
                    )
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
                    detail,
                    project_label,
                    remote_file_sha512,
                    identity_match,
                };
            }
            Some(format!(
                "Modrinth returned a candidate but tags do not exactly match target MC {mc} and loaders [{}]",
                loaders.join(", ")
            ))
        }
        Ok(None) => modrinth_strict_miss_detail(modrinth, &scan.sha512_hex, mc, loaders).await,
        Err(e) => Some(format!("Modrinth lookup failed: {e}")),
    };

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
                    let identity_match = Some(lf.file_id == m.file_id);
                    let status = if identity_match == Some(true) {
                        ResolveStatus::UpToDate
                    } else {
                        ResolveStatus::UpdateAvailable
                    };
                    let detail = if !filename_declares_mc(&lf.file_name, mc) {
                        Some(format!(
                            "CurseForge jar name does not mention MC {mc}; double-check before playing"
                        ))
                    } else {
                        None
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
                        detail,
                        project_label: mod_title,
                        remote_file_sha512: None,
                        identity_match,
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
                    detail: Some(format!(
                        "CurseForge has no file for MC {mc} with loader {}",
                        first_loader_or_any(loaders)
                    )),
                    project_label: None,
                    remote_file_sha512: None,
                    identity_match: None,
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
                    let identity_match = cf_identity_match_from_sha1(&scan, &lf);
                    let (status, detail) = if identity_match == Some(true) {
                        (ResolveStatus::UpToDate, Some("matched via search + sha1".into()))
                    } else if identity_match == Some(false) {
                        (ResolveStatus::UpdateAvailable, Some("matched via search + sha1".into()))
                    } else {
                        (
                            ResolveStatus::Unknown,
                            Some("matched via search; no exact hash identity".into()),
                        )
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
                        detail,
                        project_label: Some(format!("{} ({})", h.name, h.slug)),
                        remote_file_sha512: None,
                        identity_match,
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
        detail: Some(default_unknown_detail(modrinth_detail.as_deref(), mc, loaders)),
        project_label: None,
        remote_file_sha512: None,
        identity_match: None,
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
        remote_file_sha512: None,
        identity_match: None,
    }
}

fn cf_identity_match_from_sha1(scan: &ScannedMod, latest: &CfFileSummary) -> Option<bool> {
    let remote_sha1 = latest.hash_sha1.as_deref()?;
    let local_sha1 = hash_file_sha1(&scan.path).ok()?;
    Some(remote_sha1.eq_ignore_ascii_case(&local_sha1))
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

fn first_loader_or_any(loaders: &[String]) -> String {
    loaders.first().cloned().unwrap_or_else(|| "any".into())
}

fn default_unknown_detail(modrinth_detail: Option<&str>, mc: &str, loaders: &[String]) -> String {
    if let Some(mr) = modrinth_detail {
        return format!("{mr}; CurseForge key missing or no match for MC {mc}/loader {}", first_loader_or_any(loaders));
    }
    format!(
        "No update source matched target MC {mc} with loader {}; CurseForge key missing or no match",
        first_loader_or_any(loaders)
    )
}

async fn modrinth_strict_miss_detail(
    modrinth: &ModrinthClient,
    hash_hex: &str,
    mc: &str,
    loaders: &[String],
) -> Option<String> {
    let local = modrinth.version_from_hash(hash_hex).await.ok().flatten()?;
    let versions = modrinth.project_versions(&local.project_id, 40).await.ok()?;
    let mut tags = collect_tags_for_loaders(&versions, loaders);
    if tags.is_empty() {
        tags = collect_tags_for_loaders(&versions, &[]);
    }
    let preview = summarize_game_version_tags(&tags, mc, 6);
    if preview.is_empty() {
        return Some(format!(
            "Modrinth strict match miss for MC {mc} (loaders: {}). No game version tags were available in recent project versions",
            loaders.join(", ")
        ));
    }
    Some(format!(
        "Modrinth strict match miss for MC {mc} (loaders: {}). Nearby/available tags: {}",
        loaders.join(", "),
        preview.join(", ")
    ))
}

fn collect_tags_for_loaders(versions: &[ModrinthVersion], loaders: &[String]) -> BTreeSet<String> {
    let mut tags = BTreeSet::new();
    for ver in versions {
        let loader_ok = loaders.is_empty()
            || ver.loaders.is_empty()
            || loaders
                .iter()
                .any(|want| ver.loaders.iter().any(|have| have.eq_ignore_ascii_case(want)));
        if !loader_ok {
            continue;
        }
        for gv in &ver.game_versions {
            tags.insert(gv.trim().to_string());
        }
    }
    tags
}

fn summarize_game_version_tags(tags: &BTreeSet<String>, target_mc: &str, limit: usize) -> Vec<String> {
    let mut ranked: Vec<(i32, String)> = tags
        .iter()
        .map(|gv| (tag_distance_score(gv, target_mc), gv.clone()))
        .collect();
    ranked.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    ranked
        .into_iter()
        .take(limit)
        .map(|(_, gv)| gv)
        .collect()
}

fn tag_distance_score(tag: &str, target_mc: &str) -> i32 {
    if tag.eq_ignore_ascii_case(target_mc) {
        return 0;
    }
    let family = mc_family(target_mc);
    if family.is_some() && mc_family(tag) == family {
        return 1;
    }
    match crate::mc_version::compare_mc_versions(tag, target_mc) {
        Some(Ordering::Equal) => 0,
        Some(Ordering::Less) | Some(Ordering::Greater) => 2,
        None => 3,
    }
}

fn mc_family(ver: &str) -> Option<(u32, u32)> {
    let mut parts = ver
        .trim()
        .split('.')
        .map(|p| p.parse::<u32>().ok());
    let major = parts.next().flatten()?;
    let minor = parts.next().flatten()?;
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_nearby_tags_first() {
        let tags = BTreeSet::from([
            "1.20.4".to_string(),
            "1.21".to_string(),
            "1.21.1".to_string(),
            "1.22".to_string(),
        ]);
        let preview = summarize_game_version_tags(&tags, "1.21.11", 3);
        assert_eq!(preview[0], "1.21");
        assert_eq!(preview[1], "1.21.1");
    }

    #[test]
    fn unknown_detail_mentions_target_loader() {
        let detail = default_unknown_detail(Some("Modrinth strict match miss"), "1.21.11", &["fabric".into()]);
        assert!(detail.contains("1.21.11"));
        assert!(detail.contains("fabric"));
    }
}
