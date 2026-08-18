//! File-based secret storage under `~/.anycode/secrets/` (mode 600 on Unix).

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;

pub fn secrets_dir() -> PathBuf {
    crate::copilot_token::anycode_home_dir().join("secrets")
}

/// Strip BOM / CR / newlines / zero-width chars from pasted API keys (Windows clipboard).
pub fn normalize_secret_token(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| {
            !c.is_whitespace()
                && !c.is_control()
                && *c != '\u{7f}'
                && !matches!(*c, '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}')
        })
        .collect()
}

pub fn store_secret(name: &str, value: &str) -> Result<String> {
    let safe = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let dir = secrets_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{safe}.txt"));
    fs::write(&path, normalize_secret_token(value))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(format!("@secrets/{safe}.txt"))
}

pub fn resolve_secret_ref(reference: &str) -> Result<Option<String>> {
    let reference = reference.trim();
    if reference.is_empty() {
        return Ok(None);
    }
    let rel = reference
        .strip_prefix('@')
        .or_else(|| reference.strip_prefix("secrets/"))
        .unwrap_or(reference);
    let path = if reference.starts_with('@') || reference.starts_with("secrets/") {
        secrets_dir().join(rel.strip_prefix("secrets/").unwrap_or(rel))
    } else if !reference.contains('/') && !reference.contains('\\') {
        let safe = reference
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        secrets_dir().join(format!("{safe}.txt"))
    } else {
        PathBuf::from(reference)
    };
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let token = normalize_secret_token(&text);
    if token.is_empty() {
        return Ok(None);
    }
    Ok(Some(token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn store_and_resolve_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        env::set_var("HOME", tmp.path());
        let dir = secrets_dir();
        assert!(dir.starts_with(tmp.path()));
        let reference = store_secret("test-provider", "sk-secret").unwrap();
        assert_eq!(reference, "@secrets/test-provider.txt");
        assert_eq!(
            resolve_secret_ref(&reference).unwrap().as_deref(),
            Some("sk-secret")
        );
        store_secret("agnes", "sk-test").unwrap();
        assert_eq!(
            resolve_secret_ref("agnes").unwrap().as_deref(),
            Some("sk-test")
        );
    }

    #[test]
    fn normalize_secret_token_strips_windows_clipboard_junk() {
        assert_eq!(normalize_secret_token(" sk-ab\r\ncd\u{200b} "), "sk-abcd");
    }
}
