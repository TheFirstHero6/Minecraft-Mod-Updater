use std::cmp::Ordering;

pub fn normalize_mc(input: &str) -> String {
    input.trim().to_ascii_lowercase()
}

pub fn mc_version_components(input: &str) -> Option<Vec<u32>> {
    let normalized = normalize_mc(input);
    if normalized.is_empty() {
        return None;
    }
    if !normalized
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.')
        || normalized.starts_with('.')
        || normalized.ends_with('.')
        || normalized.contains("..")
    {
        return None;
    }
    normalized
        .split('.')
        .map(|part| part.parse::<u32>().ok())
        .collect()
}

pub fn same_mc_version(a: &str, b: &str) -> bool {
    match (mc_version_components(a), mc_version_components(b)) {
        (Some(a), Some(b)) => a == b,
        _ => normalize_mc(a) == normalize_mc(b),
    }
}

pub fn compare_mc_versions(a: &str, b: &str) -> Option<Ordering> {
    let a = mc_version_components(a)?;
    let b = mc_version_components(b)?;
    let width = a.len().max(b.len());
    for idx in 0..width {
        let lhs = *a.get(idx).unwrap_or(&0);
        let rhs = *b.get(idx).unwrap_or(&0);
        match lhs.cmp(&rhs) {
            Ordering::Equal => {}
            other => return Some(other),
        }
    }
    Some(Ordering::Equal)
}

pub fn filename_declares_mc(name: &str, target: &str) -> bool {
    let stem = name
        .trim()
        .trim_end_matches(".jar")
        .trim_end_matches(".zip")
        .to_ascii_lowercase();
    let target = normalize_mc(target);
    if target.is_empty() {
        return false;
    }
    contains_bounded(&stem, &format!("mc{target}")) || contains_bounded(&stem, &target)
}

fn contains_bounded(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return false;
    }
    let mut offset = 0usize;
    while let Some(found) = haystack[offset..].find(needle) {
        let idx = offset + found;
        let start_ok = idx == 0
            || haystack[..idx]
                .chars()
                .last()
                .is_some_and(is_token_boundary);
        let end_idx = idx + needle.len();
        let end_ok = end_idx >= haystack.len()
            || haystack[end_idx..]
                .chars()
                .next()
                .is_some_and(is_token_boundary);
        if start_ok && end_ok {
            return true;
        }
        offset = idx + 1;
    }
    false
}

fn is_token_boundary(ch: char) -> bool {
    !ch.is_ascii_alphanumeric() && ch != '.'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_segments_strictly() {
        assert_eq!(mc_version_components("1.21.11"), Some(vec![1, 21, 11]));
        assert_eq!(mc_version_components("1.21"), Some(vec![1, 21]));
        assert_eq!(mc_version_components("1.21.x"), None);
        assert_eq!(mc_version_components("1..21"), None);
    }

    #[test]
    fn distinguishes_1211_from_12111() {
        assert!(same_mc_version("1.21.11", "1.21.11"));
        assert!(!same_mc_version("1.21.1", "1.21.11"));
        assert_eq!(
            compare_mc_versions("1.21.11", "1.21.1"),
            Some(Ordering::Greater)
        );
    }

    #[test]
    fn filename_match_is_token_bounded() {
        assert!(filename_declares_mc(
            "sodium-fabric-0.6.0+mc1.21.11.jar",
            "1.21.11"
        ));
        assert!(filename_declares_mc(
            "some-mod-1.21.11-fabric.jar",
            "1.21.11"
        ));
        assert!(!filename_declares_mc(
            "sodium-fabric-0.6.0+mc1.21.11.jar",
            "1.21.1"
        ));
    }
}
