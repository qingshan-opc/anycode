//! List OS applications that can open a path (Claude-style "Open With").

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct OpenWithApp {
    /// Display name (e.g. "WPS Office").
    pub name: String,
    /// Stable id: macOS = app bundle path; others = `"default"`.
    pub id: String,
    pub is_default: bool,
}

pub fn list_open_with_apps(path: &str) -> Result<Vec<OpenWithApp>, String> {
    let p = ensure_existing_path(path)?;
    #[cfg(target_os = "macos")]
    {
        return macos::list_apps(p);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = p;
        Ok(vec![OpenWithApp {
            name: "Default application".into(),
            id: "default".into(),
            is_default: true,
        }])
    }
}

pub fn open_path_with_app(path: &str, app_id: &str) -> Result<(), String> {
    let p = ensure_existing_path(path)?;
    let path = p.to_string_lossy();
    let app_id = app_id.trim();
    if app_id.is_empty() || app_id == "default" {
        return open_with_default(path.as_ref());
    }
    #[cfg(target_os = "macos")]
    {
        return macos::open_with(path.as_ref(), app_id);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app_id;
        open_with_default(path.as_ref())
    }
}

fn ensure_existing_path(path: &str) -> Result<&std::path::Path, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err("empty path".into());
    }
    let p = std::path::Path::new(path);
    if !p.exists() {
        return Err(format!("path does not exist: {path}"));
    }
    Ok(p)
}

fn open_with_default(path: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", path])
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = path;
        Err("open unsupported on this platform".into())
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::OpenWithApp;
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSString, NSURL};

    pub fn list_apps(path: &std::path::Path) -> Result<Vec<OpenWithApp>, String> {
        let path_str = path.to_string_lossy();
        let ns_path = NSString::from_str(path_str.as_ref());
        let file_url = NSURL::fileURLWithPath(&ns_path);
        let workspace = NSWorkspace::sharedWorkspace();
        let default_url = workspace.URLForApplicationToOpenURL(&file_url);
        let default_path = default_url
            .as_ref()
            .and_then(|u| u.path())
            .map(|s| s.to_string());

        let urls = workspace.URLsForApplicationsToOpenURL(&file_url);
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let count = urls.count();
        for i in 0..count {
            let url = urls.objectAtIndex(i);
            let Some(app_path) = url.path().map(|s| s.to_string()) else {
                continue;
            };
            if !seen.insert(app_path.clone()) {
                continue;
            }
            let name = std::path::Path::new(&app_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or(app_path.as_str())
                .to_string();
            let is_default = default_path.as_ref() == Some(&app_path);
            out.push(OpenWithApp {
                name,
                id: app_path,
                is_default,
            });
        }
        out.sort_by(|a, b| match (a.is_default, b.is_default) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });
        Ok(out)
    }

    pub fn open_with(path: &str, app_bundle_path: &str) -> Result<(), String> {
        let status = std::process::Command::new("open")
            .args(["-a", app_bundle_path, path])
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("open -a failed with status {status}"))
        }
    }
}
