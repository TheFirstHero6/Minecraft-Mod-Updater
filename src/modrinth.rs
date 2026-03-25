use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use thiserror::Error;
use tokio::time::{sleep, Duration};

const BASE: &str = "https://api.modrinth.com/v2";

#[derive(Debug, Error)]
pub enum ModrinthError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("modrinth api error {status}: {body}")]
    Api { status: u16, body: String },
    #[error("invalid user-agent string")]
    InvalidUserAgent,
}

#[derive(Clone)]
pub struct ModrinthClient {
    client: reqwest::Client,
}

#[derive(Debug, Deserialize)]
pub struct ModrinthVersion {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    #[serde(default)]
    pub version_type: Option<String>,
    pub files: Vec<ModrinthFile>,
}

#[derive(Debug, Deserialize)]
pub struct ModrinthFile {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub primary: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ModrinthProject {
    pub id: String,
    pub title: String,
    pub slug: String,
}

impl ModrinthClient {
    pub fn new(user_agent: &str) -> Result<Self, ModrinthError> {
        let mut headers = HeaderMap::new();
        let ua = HeaderValue::from_str(user_agent).map_err(|_| ModrinthError::InvalidUserAgent)?;
        headers.insert(USER_AGENT, ua);
        let client = reqwest::Client::builder()
            .default_headers(headers)
            .build()?;
        Ok(Self { client })
    }

    /// Latest version for this file hash matching loaders + game versions. Returns `Ok(None)` on 404.
    pub async fn version_from_hash_update(
        &self,
        hash_hex: &str,
        loaders: &[String],
        game_versions: &[String],
    ) -> Result<Option<ModrinthVersion>, ModrinthError> {
        let url = format!("{}/version_file/{}/update?algorithm=sha512", BASE, hash_hex);
        let body = serde_json::json!({
            "loaders": loaders,
            "game_versions": game_versions,
        });
        let mut attempt = 0u32;
        let resp = loop {
            let r = self.client.post(&url).json(&body).send().await?;
            if r.status() == reqwest::StatusCode::TOO_MANY_REQUESTS && attempt < 6 {
                let ms = 400u64 * (1 << attempt.min(4));
                attempt += 1;
                sleep(Duration::from_millis(ms)).await;
                continue;
            }
            break r;
        };
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ModrinthError::Api { status, body });
        }
        let v: ModrinthVersion = resp.json().await?;
        Ok(Some(v))
    }

    pub async fn get_project(&self, project_id: &str) -> Result<ModrinthProject, ModrinthError> {
        let url = format!("{}/project/{}", BASE, project_id);
        let resp = self.client.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(ModrinthError::Api { status, body });
        }
        Ok(resp.json().await?)
    }
}
