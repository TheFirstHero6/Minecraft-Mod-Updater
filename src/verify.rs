use std::io::Read;
use std::path::Path;

use serde_json::Value as JsonValue;
use zip::ZipArchive;

use crate::mc_version::{compare_mc_versions, same_mc_version};

/// After download: ensure the JAR declares a Minecraft dependency compatible with `target_mc`.
/// Returns `Ok(())` if no declarable metadata found (cannot verify) or if compatible.
/// Returns `Err` on clear mismatch only.
pub fn verify_update_jar(path: &Path, target_mc: &str, _loaders: &[String]) -> Result<(), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let reader = std::io::BufReader::new(file);
    let mut archive = ZipArchive::new(reader).map_err(|e| e.to_string())?;

    if let Some(text) = read_zip_entry(&mut archive, "fabric.mod.json") {
        return verify_fabric_json(&text, target_mc);
    }
    if let Some(text) = read_zip_entry(&mut archive, "quilt.mod.json") {
        return verify_quilt_json(&text, target_mc);
    }
    for p in ["META-INF/mods.toml", "META-INF/neoforge.mods.toml"] {
        if let Some(text) = read_zip_entry(&mut archive, p) {
            return verify_mods_toml_minecraft(&text, target_mc);
        }
    }
    Ok(())
}

fn read_zip_entry<R: Read + std::io::Seek>(
    archive: &mut ZipArchive<R>,
    name: &str,
) -> Option<String> {
    let mut file = archive.by_name(name).ok()?;
    if file.is_dir() {
        return None;
    }
    let mut buf = Vec::with_capacity(file.size() as usize);
    std::io::Read::read_to_end(&mut file, &mut buf).ok()?;
    String::from_utf8(buf).ok()
}

fn verify_fabric_json(text: &str, target_mc: &str) -> Result<(), String> {
    let root: JsonValue = serde_json::from_str(text).map_err(|e| e.to_string())?;
    if let Some(deps) = root.get("depends") {
        if let Some(mc) = deps.get("minecraft") {
            let ok = fabric_minecraft_value_allows(mc, target_mc)?;
            if !ok {
                return Err(format!(
                    "fabric.mod.json minecraft dependency does not allow target {target_mc}"
                ));
            }
        }
    }
    Ok(())
}

fn verify_quilt_json(text: &str, target_mc: &str) -> Result<(), String> {
    let root: JsonValue = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let deps = root
        .get("quilt_loader")
        .and_then(|q| q.get("depends"))
        .or_else(|| root.get("depends"));
    if let Some(deps) = deps {
        match deps {
            JsonValue::Object(o) => {
                if let Some(mc) = o.get("minecraft") {
                    let ok = fabric_minecraft_value_allows(mc, target_mc)?;
                    if !ok {
                        return Err(format!(
                            "quilt.mod.json minecraft dependency does not allow target {target_mc}"
                        ));
                    }
                }
            }
            JsonValue::Array(items) => {
                for item in items {
                    let id = item.get("id").and_then(|x| x.as_str());
                    if id == Some("minecraft") {
                        if let Some(ver) = item.get("versions").or_else(|| item.get("version")) {
                            let ok = fabric_minecraft_value_allows(ver, target_mc)?;
                            if !ok {
                                return Err(format!(
                                    "quilt.mod.json minecraft dependency does not allow target {target_mc}"
                                ));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn fabric_minecraft_value_allows(v: &JsonValue, target_mc: &str) -> Result<bool, String> {
    match v {
        JsonValue::String(s) => Ok(minecraft_dep_allows(s, target_mc)),
        JsonValue::Array(items) => {
            if items.is_empty() {
                return Ok(true);
            }
            let ok = items.iter().any(|x| {
                x.as_str()
                    .map(|s| minecraft_dep_allows(s, target_mc))
                    .unwrap_or(false)
            });
            Ok(ok)
        }
        JsonValue::Object(_) => Ok(true),
        _ => Ok(true),
    }
}

fn verify_mods_toml_minecraft(text: &str, target_mc: &str) -> Result<(), String> {
    let v: toml::Value = toml::from_str(text).map_err(|e| e.to_string())?;
    scan_toml_tables_for_minecraft(&v, target_mc)
}

fn scan_toml_tables_for_minecraft(v: &toml::Value, target_mc: &str) -> Result<(), String> {
    match v {
        toml::Value::Table(t) => {
            if t
                .get("modId")
                .and_then(|x| x.as_str())
                .is_some_and(|id| id.eq_ignore_ascii_case("minecraft"))
            {
                if let Some(r) = t.get("versionRange").and_then(|x| x.as_str()) {
                    if !r.is_empty() && !forge_version_range_allows(r, target_mc) {
                        return Err(format!(
                            "mods.toml minecraft versionRange {r:?} does not allow {target_mc}"
                        ));
                    }
                }
            }
            for val in t.values() {
                scan_toml_tables_for_minecraft(val, target_mc)?;
            }
        }
        toml::Value::Array(a) => {
            for x in a {
                scan_toml_tables_for_minecraft(x, target_mc)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Parses common Fabric / Quilt `depends.minecraft` strings.
pub fn minecraft_dep_allows(req: &str, target: &str) -> bool {
    let req = req.trim().trim_end_matches('-');
    let target = target.trim();
    if req.is_empty() || req == "*" {
        return true;
    }
    if same_mc_version(req, target) {
        return true;
    }
    if let Some(rest) = req.strip_prefix(">=") {
        return compare_mc_versions(target, rest.trim().trim_end_matches('-'))
            .is_some_and(|o| o != std::cmp::Ordering::Less);
    }
    if let Some(rest) = req.strip_prefix('>') {
        return compare_mc_versions(target, rest.trim()) == Some(std::cmp::Ordering::Greater);
    }
    if let Some(rest) = req.strip_prefix("<=") {
        return compare_mc_versions(target, rest.trim())
            .is_some_and(|o| o != std::cmp::Ordering::Greater);
    }
    if let Some(rest) = req.strip_prefix('<') {
        return compare_mc_versions(target, rest.trim()) == Some(std::cmp::Ordering::Less);
    }
    if let Some(rest) = req.strip_prefix('~') {
        return tilde_allows(rest.trim(), target);
    }
    if req.starts_with('[') || req.starts_with('(') {
        return maven_interval_allows(req, target);
    }
    if req.contains(',') && (req.contains('[') || req.contains('(')) {
        return maven_interval_allows(req, target);
    }
    same_mc_version(req, target)
}

fn forge_version_range_allows(range: &str, target: &str) -> bool {
    maven_interval_allows(range, target)
}

fn maven_interval_allows(spec: &str, target: &str) -> bool {
    let s = spec.trim();
    if !s.starts_with(['[', '(']) {
        return minecraft_dep_allows(s, target);
    }
    let left_inclusive = s.starts_with('[');
    let right_inclusive = s.ends_with(']');
    let inner = s
        .trim_start_matches(['[', '('])
        .trim_end_matches([']', ')']);
    let parts: Vec<&str> = inner
        .split(',')
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .collect();
    if parts.is_empty() {
        return true;
    }
    let low = parts[0];
    let high = parts.get(1).copied().unwrap_or("");
    if !low.is_empty() {
        let Some(c) = compare_mc_versions(target, low) else {
            return false;
        };
        if c == std::cmp::Ordering::Less || (c == std::cmp::Ordering::Equal && !left_inclusive) {
            return false;
        }
    }
    if !high.is_empty() {
        let Some(c) = compare_mc_versions(target, high) else {
            return false;
        };
        if c == std::cmp::Ordering::Greater || (c == std::cmp::Ordering::Equal && !right_inclusive)
        {
            return false;
        }
    }
    true
}

fn tilde_allows(base: &str, target: &str) -> bool {
    let Some(b) = crate::mc_version::mc_version_components(base) else {
        return false;
    };
    let Some(t) = crate::mc_version::mc_version_components(target) else {
        return false;
    };
    if b.len() < 2 || t.len() < 2 {
        return compare_mc_versions(target, base).is_some_and(|o| o != std::cmp::Ordering::Less);
    }
    b[0] == t[0]
        && b[1] == t[1]
        && compare_mc_versions(target, base).is_some_and(|o| o != std::cmp::Ordering::Less)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mc_compare() {
        assert_eq!(
            compare_mc_versions("1.21.1", "1.21.0"),
            Some(std::cmp::Ordering::Greater)
        );
        assert_eq!(
            compare_mc_versions("1.21.1", "1.21.1"),
            Some(std::cmp::Ordering::Equal)
        );
        assert_eq!(
            compare_mc_versions("1.20.4", "1.21.1"),
            Some(std::cmp::Ordering::Less)
        );
    }

    #[test]
    fn fabric_dep_tilde() {
        assert!(minecraft_dep_allows("~1.21.1", "1.21.1"));
        assert!(minecraft_dep_allows("~1.21.1", "1.21.5"));
        assert!(!minecraft_dep_allows("~1.21.1", "1.22.0"));
    }

    #[test]
    fn fabric_dep_ge() {
        assert!(minecraft_dep_allows(">=1.21", "1.21.1"));
        assert!(!minecraft_dep_allows(">=1.22", "1.21.1"));
    }

    #[test]
    fn maven_interval() {
        assert!(maven_interval_allows("[1.21,1.22)", "1.21.1"));
        assert!(!maven_interval_allows("[1.21,1.22)", "1.22.0"));
    }

    #[test]
    fn bare_versions_are_not_prefix_matched() {
        assert!(!minecraft_dep_allows("1.21.1", "1.21.11"));
        assert!(!minecraft_dep_allows("1.21.11", "1.21.1"));
        assert!(minecraft_dep_allows("[1.21.11,1.21.12)", "1.21.11"));
        assert!(!minecraft_dep_allows("[1.21.1,1.21.2)", "1.21.11"));
    }
}
