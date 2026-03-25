use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value as JsonValue;
use sha1::Sha1;
use sha2::{Digest, Sha512};
use thiserror::Error;
use zip::ZipArchive;

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("invalid utf-8 in {0}")]
    Utf8(String),
}

#[derive(Debug, Clone)]
pub struct ModMetadata {
    pub id: String,
    pub version: String,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub struct ScannedMod {
    pub path: PathBuf,
    pub file_name: String,
    pub sha512_hex: String,
    pub metadata: Option<ModMetadata>,
}

fn io_err(path: &Path, e: std::io::Error) -> ScanError {
    ScanError::Io {
        path: path.to_path_buf(),
        source: e,
    }
}

pub fn hash_file_sha512(path: &Path) -> Result<String, ScanError> {
    let file = File::open(path).map_err(|e| io_err(path, e))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha512::new();
    let mut buf = [0u8; 65_536];
    loop {
        let n = reader.read(&mut buf).map_err(|e| io_err(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

pub fn hash_file_sha1(path: &Path) -> Result<String, ScanError> {
    let file = File::open(path).map_err(|e| io_err(path, e))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha1::new();
    let mut buf = [0u8; 65_536];
    loop {
        let n = reader.read(&mut buf).map_err(|e| io_err(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

#[derive(Debug, Deserialize)]
struct FabricModJson {
    id: String,
    version: String,
    #[serde(default)]
    name: Option<String>,
}

fn read_zip_entry(archive: &mut ZipArchive<BufReader<File>>, name: &str) -> Option<String> {
    let Ok(mut file) = archive.by_name(name) else {
        return None;
    };
    if file.is_dir() {
        return None;
    }
    let mut buf = Vec::with_capacity(file.size() as usize);
    if std::io::Read::read_to_end(&mut file, &mut buf).is_err() {
        return None;
    }
    String::from_utf8(buf).ok()
}

fn parse_fabric_like(json: &str) -> Option<ModMetadata> {
    let v: FabricModJson = serde_json::from_str(json).ok()?;
    let display = v.name.unwrap_or_else(|| v.id.clone());
    Some(ModMetadata {
        id: v.id,
        version: v.version,
        display_name: display,
    })
}

#[derive(Debug, Deserialize)]
struct ModsToml {
    #[serde(default)]
    mods: Vec<ModsTomlEntry>,
}

#[derive(Debug, Deserialize)]
struct ModsTomlEntry {
    #[serde(rename = "modId")]
    mod_id: String,
    version: String,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(rename = "displayName")]
    display_name_camel: Option<String>,
}

fn parse_mods_toml(text: &str) -> Option<ModMetadata> {
    let parsed: ModsToml = toml::from_str(text).ok()?;
    let m = parsed.mods.first()?;
    let display = m
        .display_name
        .clone()
        .or(m.display_name_camel.clone())
        .unwrap_or_else(|| m.mod_id.clone());
    Some(ModMetadata {
        id: m.mod_id.clone(),
        version: m.version.clone(),
        display_name: display,
    })
}

/// Try `META-INF/mods.toml` then NeoForge path.
fn try_mods_toml(archive: &mut ZipArchive<BufReader<File>>) -> Option<ModMetadata> {
    for path in ["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
        if let Some(text) = read_zip_entry(archive, path) {
            if let Some(m) = parse_mods_toml(&text) {
                return Some(m);
            }
        }
    }
    None
}

fn try_fabric_quilt(archive: &mut ZipArchive<BufReader<File>>) -> Option<ModMetadata> {
    for path in ["fabric.mod.json", "quilt.mod.json"] {
        if let Some(text) = read_zip_entry(archive, path) {
            if path == "quilt.mod.json" {
                if let Some(m) = parse_quilt_mod_json(&text) {
                    return Some(m);
                }
            } else if let Some(m) = parse_fabric_like(&text) {
                return Some(m);
            }
        }
    }
    None
}

fn parse_quilt_mod_json(text: &str) -> Option<ModMetadata> {
    let root: JsonValue = serde_json::from_str(text).ok()?;
    let id = root.get("quilt_loader")?.get("id")?.as_str()?.to_string();
    let version = root
        .get("quilt_loader")?
        .get("version")?
        .as_str()?
        .to_string();
    let display = root
        .get("quilt_loader")?
        .get("metadata")
        .and_then(|m| m.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or(&id)
        .to_string();
    Some(ModMetadata {
        id,
        version,
        display_name: display,
    })
}

pub fn scan_jar(path: &Path) -> Result<ScannedMod, ScanError> {
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sha512_hex = hash_file_sha512(path)?;
    let file = File::open(path).map_err(|e| io_err(path, e))?;
    let reader = BufReader::new(file);
    let mut archive = ZipArchive::new(reader)?;
    let metadata = try_fabric_quilt(&mut archive).or_else(|| try_mods_toml(&mut archive));
    Ok(ScannedMod {
        path: path.to_path_buf(),
        file_name,
        sha512_hex,
        metadata,
    })
}

pub fn scan_mods_dir(dir: &Path) -> Result<Vec<ScannedMod>, ScanError> {
    let mut out = Vec::new();
    let rd = std::fs::read_dir(dir).map_err(|e| io_err(dir, e))?;
    for ent in rd {
        let ent = ent.map_err(|e| io_err(dir, e))?;
        let p = ent.path();
        if p.extension().is_some_and(|e| e == "jar") {
            out.push(scan_jar(&p)?);
        }
    }
    out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::{SimpleFileOptions, ZipWriter};

    fn write_test_zip(path: &Path, inner_name: &str, inner: &[u8]) {
        let file = File::create(path).unwrap();
        let mut zip = ZipWriter::new(file);
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        zip.start_file(inner_name, opts).unwrap();
        zip.write_all(inner).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn fabric_metadata_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("test.jar");
        let fabric = r#"{"schemaVersion":1,"id":"testmod","version":"1.2.3","name":"Test Mod"}"#;
        write_test_zip(&jar, "fabric.mod.json", fabric.as_bytes());
        let m = scan_jar(&jar).unwrap();
        assert_eq!(m.metadata.as_ref().unwrap().id, "testmod");
        assert_eq!(m.metadata.as_ref().unwrap().version, "1.2.3");
        assert!(!m.sha512_hex.is_empty());
    }

    #[test]
    fn mods_toml_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("forge.jar");
        let toml_txt = r#"
modLoader="javafml"
loaderVersion="[47,)"

[[mods]]
modId="forgemod"
version="2.0.0"
displayName="Forge Mod"
"#;
        write_test_zip(&jar, "META-INF/mods.toml", toml_txt.as_bytes());
        let m = scan_jar(&jar).unwrap();
        assert_eq!(m.metadata.as_ref().unwrap().id, "forgemod");
        assert_eq!(m.metadata.as_ref().unwrap().version, "2.0.0");
    }
}
