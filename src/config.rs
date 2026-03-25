use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config: {0}")]
    Read(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("minecraft_version is required")]
    MissingMinecraftVersion,
    #[error("at least one loader is required")]
    MissingLoaders,
    #[error("mods_dir is required")]
    MissingModsDir,
    #[error("invalid mods_dir: {0}")]
    InvalidModsDir(String),
    #[error("invalid loader: {0} (use fabric, forge, neoforge, quilt)")]
    InvalidLoader(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadConfig {
    #[serde(default = "default_backup")]
    pub backup: bool,
    #[serde(default)]
    pub backup_dir: Option<PathBuf>,
    #[serde(default)]
    pub dry_run: bool,
}

fn default_backup() -> bool {
    true
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            backup: true,
            backup_dir: None,
            dry_run: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Directory containing `.jar` mods (required unless overridden by CLI).
    pub mods_dir: Option<PathBuf>,
    /// Target Minecraft version, e.g. `1.21.1`.
    pub minecraft_version: Option<String>,
    /// Mod loaders to query (e.g. fabric, forge).
    pub loaders: Vec<String>,
    /// Modrinth User-Agent (see https://docs.modrinth.com/api).
    pub user_agent: Option<String>,
    #[serde(default)]
    pub curseforge_api_key: Option<String>,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default)]
    pub download: DownloadConfig,
}

fn default_concurrency() -> usize {
    8
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mods_dir: None,
            minecraft_version: None,
            loaders: Vec::new(),
            user_agent: None,
            curseforge_api_key: None,
            concurrency: default_concurrency(),
            download: DownloadConfig::default(),
        }
    }
}

impl Config {
    pub fn default_config_path() -> Option<PathBuf> {
        directories::BaseDirs::new().map(|b| b.config_dir().join("mod-updater").join("config.toml"))
    }

    pub fn load_from_path(path: &Path) -> Result<Self, ConfigError> {
        let raw = std::fs::read_to_string(path)?;
        let c: Config = toml::from_str(&raw)?;
        Ok(c)
    }

    pub fn validate(self) -> Result<Self, ConfigError> {
        let mods_dir = self.mods_dir.clone().ok_or(ConfigError::MissingModsDir)?;
        if !mods_dir.is_dir() {
            return Err(ConfigError::InvalidModsDir(format!(
                "{} is not a directory",
                mods_dir.display()
            )));
        }
        let minecraft_version = self
            .minecraft_version
            .clone()
            .ok_or(ConfigError::MissingMinecraftVersion)?;
        if minecraft_version.is_empty() {
            return Err(ConfigError::MissingMinecraftVersion);
        }
        if self.loaders.is_empty() {
            return Err(ConfigError::MissingLoaders);
        }
        for l in &self.loaders {
            let n = normalize_loader(l);
            if !matches!(
                n.as_str(),
                "fabric" | "forge" | "neoforge" | "quilt" | "liteloader" | "cauldron"
            ) {
                return Err(ConfigError::InvalidLoader(l.clone()));
            }
        }
        Ok(Self {
            mods_dir: Some(mods_dir),
            minecraft_version: Some(minecraft_version),
            ..self
        })
    }

    pub fn mods_dir(&self) -> &Path {
        self.mods_dir.as_ref().expect("validated")
    }

    pub fn minecraft_version(&self) -> &str {
        self.minecraft_version.as_ref().expect("validated")
    }

    pub fn normalized_loaders(&self) -> Vec<String> {
        self.loaders.iter().map(|s| normalize_loader(s)).collect()
    }

    pub fn user_agent(&self) -> String {
        self.user_agent
            .clone()
            .unwrap_or_else(|| format!("mod-updater/{}", env!("CARGO_PKG_VERSION")))
    }

    pub fn resolved_backup_dir(&self) -> PathBuf {
        if let Some(ref p) = self.download.backup_dir {
            expand_home(p)
        } else if let Some(proj) =
            directories::ProjectDirs::from("org", "mod-updater", "mod-updater")
        {
            proj.data_local_dir().join("backups")
        } else {
            PathBuf::from(".mod-updater-backups")
        }
    }
}

pub fn normalize_loader(s: &str) -> String {
    s.trim().to_ascii_lowercase()
}

fn expand_home(p: &Path) -> PathBuf {
    if let Some(st) = p.to_str() {
        if let Some(rest) = st.strip_prefix("~/") {
            if let Some(h) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) {
                return h.join(rest);
            }
        }
    }
    p.to_path_buf()
}
