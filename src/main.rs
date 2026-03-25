use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use mod_updater::config::Config;
use mod_updater::scan::scan_mods_dir;
use mod_updater::tui;

#[derive(Parser, Debug)]
#[command(name = "mod-updater")]
#[command(about = "Check and update Minecraft mods (Modrinth + optional CurseForge)")]
struct Cli {
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    mods_dir: Option<PathBuf>,
    #[arg(long)]
    minecraft_version: Option<String>,
    #[arg(long, value_delimiter = ',')]
    loaders: Option<Vec<String>>,
    #[arg(long, help = "Required by Modrinth; include contact if possible")]
    user_agent: Option<String>,
    #[arg(long)]
    curseforge_api_key: Option<String>,
}

fn load_merged(cli: &Cli) -> anyhow::Result<Config> {
    let mut cfg = if let Some(p) = &cli.config {
        Config::load_from_path(p).map_err(|e| anyhow::anyhow!(e))?
    } else if let Some(def) = Config::default_config_path() {
        if def.exists() {
            Config::load_from_path(&def).map_err(|e| anyhow::anyhow!(e))?
        } else {
            Config::default()
        }
    } else {
        Config::default()
    };

    if let Some(d) = &cli.mods_dir {
        cfg.mods_dir = Some(d.clone());
    }
    if let Some(v) = &cli.minecraft_version {
        cfg.minecraft_version = Some(v.clone());
    }
    if let Some(loaders) = &cli.loaders {
        if !loaders.is_empty() {
            cfg.loaders = loaders.clone();
        }
    }
    if let Some(ua) = &cli.user_agent {
        cfg.user_agent = Some(ua.clone());
    }
    if let Some(k) = &cli.curseforge_api_key {
        cfg.curseforge_api_key = Some(k.clone());
    }

    if cfg.mods_dir.is_none() {
        if let Ok(p) = std::env::var("MOD_UPDATER_MODS_DIR") {
            let p = p.trim();
            if !p.is_empty() {
                cfg.mods_dir = Some(PathBuf::from(p));
            }
        }
    }

    cfg.validate().map_err(|e| {
        let path_hint = Config::default_config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "~/.config/mod-updater/config.toml".into());
        anyhow::anyhow!(
            "{e}\n\nSet mods_dir in {path_hint}, or pass --mods-dir, or set env MOD_UPDATER_MODS_DIR.\nExample TOML:\n  mods_dir = \"/path/to/.minecraft/mods\"\n  minecraft_version = \"1.21.1\"\n  loaders = [\"fabric\"]\n  user_agent = \"you/mod-updater/0.1 (email@domain)\""
        )
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut cli = Cli::parse();
    if cli.curseforge_api_key.is_none() {
        cli.curseforge_api_key = std::env::var("CURSEFORGE_API_KEY")
            .ok()
            .filter(|s| !s.is_empty());
    }
    let config = Arc::new(load_merged(&cli).context("config")?);
    let scans = scan_mods_dir(config.mods_dir()).context("scan mods directory")?;
    tui::run(config, scans).await
}
