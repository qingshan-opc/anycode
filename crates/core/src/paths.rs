//! User home and `~/.anycode` resolution.
//!
//! Windows typically has `USERPROFILE` but not `HOME`. Callers that used
//! `std::env::var("HOME")?` failed with "environment variable not found".

use std::path::PathBuf;

/// User home: `HOME` if set, otherwise `dirs::home_dir()` (`USERPROFILE` on Windows).
#[must_use]
pub fn user_home_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
}

/// `~/.anycode`, or `$ANYCODE_HOME` when that env is a non-empty path.
#[must_use]
pub fn anycode_data_dir() -> Option<PathBuf> {
    let from_env = std::env::var("ANYCODE_HOME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    if from_env.is_some() {
        return from_env;
    }
    user_home_dir().map(|h| h.join(".anycode"))
}

/// Same as [`anycode_data_dir`], falling back to `./.anycode` when no home exists.
#[must_use]
pub fn anycode_data_dir_or_cwd() -> PathBuf {
    anycode_data_dir().unwrap_or_else(|| PathBuf::from(".anycode"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_home_dir_resolves_without_unix_home_semantics() {
        assert!(
            user_home_dir().is_some(),
            "dirs::home_dir / HOME / USERPROFILE should resolve on this host"
        );
    }

    #[test]
    fn anycode_data_dir_is_under_home_or_override() {
        let dir = anycode_data_dir().expect("data dir");
        if std::env::var("ANYCODE_HOME")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some()
        {
            assert_eq!(dir, PathBuf::from(std::env::var("ANYCODE_HOME").unwrap()));
        } else {
            assert!(dir.ends_with(".anycode"));
        }
    }
}
