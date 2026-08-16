//! Skill App surface contract (`ui/surface.yaml` + VisualBrief).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Host slot where a Skill App may mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillAppSlot {
    Dock,
    Conversation,
    Project,
}

impl SkillAppSlot {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dock => "dock",
            Self::Conversation => "conversation",
            Self::Project => "project",
        }
    }
}

/// How long the mini-app runtime state is retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SkillAppLifecycle {
    Ephemeral,
    #[default]
    Session,
    Persistent,
}

/// Parsed `ui/surface.yaml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillSurface {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default = "default_entry")]
    pub entry: String,
    #[serde(default = "default_slots")]
    pub slots: Vec<SkillAppSlot>,
    #[serde(default = "default_slot_dock")]
    pub default_slot: SkillAppSlot,
    #[serde(default)]
    pub lifecycle: SkillAppLifecycle,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub permissions: SkillSurfacePermissions,
}

fn default_entry() -> String {
    "index.html".into()
}

fn default_slots() -> Vec<SkillAppSlot> {
    vec![SkillAppSlot::Dock]
}

fn default_slot_dock() -> SkillAppSlot {
    SkillAppSlot::Dock
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SkillSurfacePermissions {
    /// Host bridge methods the app may call (e.g. `state`, `brief.submit`).
    #[serde(default)]
    pub host_api: Vec<String>,
    #[serde(default)]
    pub network: bool,
}

/// User-locked visual contract returned via `brief.submit`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualBrief {
    pub skill_id: String,
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub templates: Vec<String>,
    #[serde(default)]
    pub tokens: serde_json::Value,
    #[serde(default)]
    pub density: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Opaque app-specific payload.
    #[serde(default)]
    pub extra: serde_json::Value,
}

impl Default for VisualBrief {
    fn default() -> Self {
        Self {
            skill_id: String::new(),
            schema: "visual-brief/v1".into(),
            family: None,
            templates: Vec::new(),
            tokens: serde_json::json!({}),
            density: None,
            notes: None,
            extra: serde_json::json!({}),
        }
    }
}

/// Resolve `ui/` directory for a skill root (if present).
pub fn skill_ui_dir(skill_root: &Path) -> Option<PathBuf> {
    let ui = skill_root.join("ui");
    if !ui.is_dir() {
        return None;
    }
    if ui.join("index.html").is_file() || ui.join("surface.yaml").is_file() {
        Some(ui)
    } else {
        None
    }
}

/// Load surface from skill root. Returns `None` when no UI is declared.
pub fn load_skill_surface(skill_root: &Path) -> Option<SkillSurface> {
    let ui = skill_ui_dir(skill_root)?;
    let yaml_path = ui.join("surface.yaml");
    if yaml_path.is_file() {
        let text = fs::read_to_string(&yaml_path).ok()?;
        return parse_surface_yaml(&text);
    }
    // Implicit surface when only index.html exists.
    let skill_id = skill_root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("skill")
        .to_string();
    if ui.join("index.html").is_file() {
        Some(SkillSurface {
            id: format!("{skill_id}-app"),
            title: skill_id,
            entry: "index.html".into(),
            slots: default_slots(),
            default_slot: SkillAppSlot::Dock,
            lifecycle: SkillAppLifecycle::Session,
            icon: None,
            permissions: SkillSurfacePermissions {
                host_api: vec!["state".into(), "brief.submit".into(), "agent.prompt".into()],
                network: false,
            },
        })
    } else {
        None
    }
}

pub fn parse_surface_yaml(text: &str) -> Option<SkillSurface> {
    serde_yaml::from_str(text).ok()
}

/// Whether `rel` stays inside `ui/` (no `..` escape).
pub fn resolve_ui_asset(ui_dir: &Path, rel: &str) -> Option<PathBuf> {
    let rel = rel.trim_start_matches('/');
    if rel.is_empty() || rel.contains('\0') {
        return None;
    }
    let candidate = ui_dir.join(rel);
    let canon_ui = ui_dir.canonicalize().ok()?;
    let canon = candidate.canonicalize().ok().or_else(|| {
        // Allow missing? No — only existing files.
        None
    })?;
    if canon.starts_with(&canon_ui) {
        Some(canon)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_surface_minimal() {
        let s = parse_surface_yaml(
            r#"
id: hello-app
title: Hello
entry: index.html
slots: [dock, conversation, project]
default_slot: dock
lifecycle: persistent
permissions:
  host_api: [state, brief.submit]
  network: false
"#,
        )
        .expect("parse");
        assert_eq!(s.id, "hello-app");
        assert_eq!(s.slots.len(), 3);
        assert_eq!(s.default_slot, SkillAppSlot::Dock);
        assert!(!s.permissions.network);
    }

    #[test]
    fn visual_brief_roundtrip() {
        let b = VisualBrief {
            skill_id: "anycode-ppt".into(),
            schema: "visual-brief/v1".into(),
            family: Some("fde-editorial".into()),
            templates: vec!["cover".into(), "section".into()],
            tokens: serde_json::json!({"accent": "#1400ff"}),
            density: Some("diagram".into()),
            notes: None,
            extra: serde_json::json!({}),
        };
        let v = serde_json::to_value(&b).unwrap();
        let back: VisualBrief = serde_json::from_value(v).unwrap();
        assert_eq!(back.templates.len(), 2);
    }
}
