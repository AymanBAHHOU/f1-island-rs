//! Version numbers like "v0.1.10": parsing them and comparing by number, not by text.
//! (Port of `parseVersion` / `isNewer` in the Electron app's `src/updater.js`.)

/// "v0.1.10" or "0.1.10-beta.1" gives `Some([0, 1, 10])`; anything that doesn't
/// start with three dot-separated numbers gives `None`.
///
/// `Option<T>` is how Rust says "there might be nothing here": the compiler makes
/// every caller deal with the `None` case, so "not a version" can't be forgotten.
pub fn parse_version(text: &str) -> Option<[u64; 3]> {
    let text = text.trim();
    let mut rest = text.strip_prefix('v').unwrap_or(text);

    let mut parts = [0u64; 3];
    for (i, part) in parts.iter_mut().enumerate() {
        // Take the run of digits at the start of what's left.
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        // `?` means: if this is None (or an error), return it from the function now.
        *part = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        // After the first two numbers there must be a dot.
        if i < 2 {
            rest = rest.strip_prefix('.')?;
        }
    }
    Some(parts)
}

/// Is `latest` a newer version than `current`? Anything that isn't a version counts as "no".
pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        // Arrays compare element by element, left to right: [0, 1, 10] > [0, 1, 9].
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions_with_or_without_v_and_suffixes() {
        assert_eq!(parse_version("v0.1.10"), Some([0, 1, 10]));
        assert_eq!(parse_version("0.1.10-beta.1"), Some([0, 1, 10]));
        assert_eq!(parse_version("  v1.2.3  "), Some([1, 2, 3]));
        assert_eq!(parse_version("1.2.3.4"), Some([1, 2, 3]));
    }

    #[test]
    fn rejects_things_that_are_not_versions() {
        assert_eq!(parse_version("nightly"), None);
        assert_eq!(parse_version(""), None);
        assert_eq!(parse_version("1.2"), None);
        assert_eq!(parse_version("1..3"), None);
        assert_eq!(parse_version("v"), None);
    }

    #[test]
    fn compares_by_number_not_by_text() {
        assert!(is_newer("0.1.10", "0.1.9")); // as text, "0.1.10" < "0.1.9"
        assert!(is_newer("v0.2.0", "0.1.99"));
        assert!(is_newer("1.0.0-beta.1", "0.9.9"));
        assert!(!is_newer("0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.0", "0.1.1"));
        assert!(!is_newer("garbage", "0.1.0"));
        assert!(!is_newer("0.1.0", "garbage"));
    }
}
