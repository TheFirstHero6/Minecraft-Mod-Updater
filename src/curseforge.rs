use std::fs::File;
use std::io::Read;
use std::path::Path;

use reqwest::header::{HeaderMap, HeaderValue, ACCEPT};
use serde_json::Value;
use thiserror::Error;

const BASE: &str = "https://api.curseforge.com/v1";
pub const MINECRAFT_GAME_ID: i32 = 432;

#[derive(Debug, Error)]
pub enum CurseForgeError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("curseforge api error {status}: {body}")]
    Api { status: u16, body: String },
    #[error("invalid api key header")]
    InvalidKey,
}

/// CurseForge `modLoader` field on files. See API ModLoaderType.
#[derive(Debug, Clone, Copy)]
pub enum CfModLoader {
    Forge = 1,
    Fabric = 4,
    Quilt = 5,
    NeoForge = 6,
}

impl CfModLoader {
    pub fn from_loader_str(s: &str) -> Option<Self> {
        match s {
            "forge" => Some(Self::Forge),
            "fabric" => Some(Self::Fabric),
            "quilt" => Some(Self::Quilt),
            "neoforge" => Some(Self::NeoForge),
            _ => None,
        }
    }
}

/// Port of meza/curseforge-fingerprint `fingerprint.cpp` (CurseForge jar fingerprint).
pub fn fingerprint_jar_bytes(buffer: &[u8]) -> u32 {
    const MULTIPLEX: u32 = 1_540_483_477;
    let num1 = compute_normalized_length(buffer);
    let mut num2 = 1u32 ^ num1;
    let mut num3 = 0u32;
    let mut num4 = 0u32;
    for &b in buffer {
        if !is_whitespace(b) {
            num3 |= (b as u32) << num4;
            num4 += 8;
            if num4 == 32 {
                let num6 = num3.wrapping_mul(MULTIPLEX);
                let num7 = (num6 ^ (num6 >> 24)).wrapping_mul(MULTIPLEX);
                num2 = (num2.wrapping_mul(MULTIPLEX)) ^ num7;
                num3 = 0;
                num4 = 0;
            }
        }
    }
    if num4 > 0 {
        num2 = (num2 ^ num3).wrapping_mul(MULTIPLEX);
    }
    let num6 = (num2 ^ (num2 >> 13)).wrapping_mul(MULTIPLEX);
    num6 ^ (num6 >> 15)
}

fn is_whitespace(b: u8) -> bool {
    matches!(b, 9 | 10 | 13 | 32)
}

fn compute_normalized_length(buffer: &[u8]) -> u32 {
    buffer.iter().filter(|&&b| !is_whitespace(b)).count() as u32
}

pub fn fingerprint_file(path: &Path) -> std::io::Result<u32> {
    let mut file = File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(fingerprint_jar_bytes(&buf))
}

#[derive(Clone)]
pub struct CurseForgeClient {
    client: reqwest::Client,
}

impl CurseForgeClient {
    pub fn new(api_key: &str) -> Result<Self, CurseForgeError> {
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let key = HeaderValue::from_str(api_key).map_err(|_| CurseForgeError::InvalidKey)?;
        headers.insert("x-api-key", key);
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;
        Ok(Self { client })
    }

    pub async fn fingerprint_matches(
        &self,
        fingerprints: &[u32],
    ) -> Result<Vec<FingerprintMatch>, CurseForgeError> {
        let url = format!("{}/fingerprints/{}", BASE, MINECRAFT_GAME_ID);
        let body = serde_json::json!({ "fingerprints": fingerprints });
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(CurseForgeError::Api { status, body });
        }
        let v: Value = resp.json().await?;
        let empty = vec![];
        let arr = v
            .get("data")
            .and_then(|d| d.get("exactMatches"))
            .and_then(|x| x.as_array())
            .unwrap_or(&empty);
        let mut out = Vec::new();
        for m in arr {
            let file = m.get("file").cloned().unwrap_or(Value::Null);
            let mod_id = file.get("modId").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            let file_id = file.get("id").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            let name = file
                .get("fileName")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let download_url = file
                .get("downloadUrl")
                .and_then(|x| x.as_str())
                .map(String::from);
            let game_versions: Vec<String> = file
                .get("gameVersions")
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let mod_loader = file.get("modLoader").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            out.push(FingerprintMatch {
                mod_id,
                file_id,
                file_name: name,
                download_url,
                game_versions,
                mod_loader,
            });
        }
        Ok(out)
    }

    pub async fn search_mods(
        &self,
        search: &str,
        page_size: u32,
    ) -> Result<Vec<CfModSearchHit>, CurseForgeError> {
        let url = format!("{}/mods/search", BASE);
        let resp = self
            .client
            .get(&url)
            .query(&[
                ("gameId", MINECRAFT_GAME_ID.to_string()),
                ("searchFilter", search.to_string()),
                ("pageSize", page_size.to_string()),
            ])
            .send()
            .await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(CurseForgeError::Api { status, body });
        }
        let v: Value = resp.json().await?;
        let empty = vec![];
        let data = v.get("data").and_then(|d| d.as_array()).unwrap_or(&empty);
        let mut hits = Vec::new();
        for item in data {
            let id = item.get("id").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
            let name = item
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let slug = item
                .get("slug")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            hits.push(CfModSearchHit { id, name, slug });
        }
        Ok(hits)
    }

    /// Fetch mod files and pick newest that supports `minecraft_version` and optional loader.
    pub async fn latest_file_for_game(
        &self,
        mod_id: i32,
        minecraft_version: &str,
        loader: Option<CfModLoader>,
    ) -> Result<Option<CfFileSummary>, CurseForgeError> {
        let url = format!("{}/mods/{}/files", BASE, mod_id);
        let mut index = 0i32;
        let page_size = 50u32;
        let mut candidates: Vec<CfFileSummary> = Vec::new();
        loop {
            let resp = self
                .client
                .get(&url)
                .query(&[
                    ("index", index.to_string()),
                    ("pageSize", page_size.to_string()),
                ])
                .send()
                .await?;
            if !resp.status().is_success() {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                return Err(CurseForgeError::Api { status, body });
            }
            let v: Value = resp.json().await?;
            let empty = vec![];
            let data = v.get("data").and_then(|d| d.as_array()).unwrap_or(&empty);
            if data.is_empty() {
                break;
            }
            for f in data {
                let file_id = f.get("id").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
                let file_name = f
                    .get("fileName")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let download_url = f
                    .get("downloadUrl")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                let game_versions: Vec<String> = f
                    .get("gameVersions")
                    .and_then(|x| x.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                if !game_versions.iter().any(|gv| gv == minecraft_version) {
                    continue;
                }
                let mod_loader = f.get("modLoader").and_then(|x| x.as_i64()).unwrap_or(0) as i32;
                if let Some(w) = loader {
                    if mod_loader != w as i32 {
                        continue;
                    }
                }
                let file_date = f
                    .get("fileDate")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let release_type = f.get("releaseType").and_then(|x| x.as_i64()).unwrap_or(0) as u8;
                candidates.push(CfFileSummary {
                    file_id,
                    file_name,
                    download_url,
                    game_versions,
                    mod_loader,
                    file_date,
                    release_type,
                });
            }
            index += data.len() as i32;
            if data.len() < page_size as usize {
                break;
            }
            if index > 5000 {
                break;
            }
        }
        candidates.sort_by(|a, b| b.file_date.cmp(&a.file_date));
        Ok(candidates.into_iter().next())
    }

    pub async fn get_mod_description(&self, mod_id: i32) -> Result<String, CurseForgeError> {
        let url = format!("{}/mods/{}", BASE, mod_id);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(CurseForgeError::Api { status, body });
        }
        let v: Value = resp.json().await?;
        Ok(v.get("data")
            .and_then(|d| d.get("name"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string())
    }
}

#[derive(Debug, Clone)]
pub struct FingerprintMatch {
    pub mod_id: i32,
    pub file_id: i32,
    pub file_name: String,
    pub download_url: Option<String>,
    pub game_versions: Vec<String>,
    pub mod_loader: i32,
}

#[derive(Debug, Clone)]
pub struct CfModSearchHit {
    pub id: i32,
    pub name: String,
    pub slug: String,
}

#[derive(Debug, Clone)]
pub struct CfFileSummary {
    pub file_id: i32,
    pub file_name: String,
    pub download_url: Option<String>,
    pub game_versions: Vec<String>,
    pub mod_loader: i32,
    pub file_date: String,
    pub release_type: u8,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_matches_meza_vectors() {
        let t1 = b"# This is the first test file\n";
        let t2 = b"# This is the second test file\n";
        assert_eq!(fingerprint_jar_bytes(t1), 3_608_199_863);
        assert_eq!(fingerprint_jar_bytes(t2), 3_493_718_775);
    }
}
