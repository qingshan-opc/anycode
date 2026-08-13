//! Scaffold a standard-contract skill directory (roadmap P0.4 authoring half).
//!
//! The skeleton bakes in everything `vet` lints for: a 30–200 char description
//! with use/when-not guidance, the four required SOP sections, and declarative
//! acceptance checks that the `Skill` tool runs after `run` exits.

use super::SkillCatalog;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

const SKILL_MD_SKELETON: &str = r#"---
name: {id}
description: {description}
category: other
version: 0.1.0
# acceptance:
#   - type: file-exists
#     path: "out/result.md"
---

# {id}

One paragraph: what this skill produces and the workflow it embeds into.

## Inputs

- `<arg0>` — what the caller must pass (paths, options).

## Steps

1. First concrete action.
2. Next action.
3. Verify the output against the acceptance checks below.

## Output

- `out/…` — exact artifact paths this skill writes.

## Acceptance Checks

- State the machine-checkable outcome, mirroring frontmatter `acceptance:`.
"#;

const RUN_SKELETON: &str = r#"#!/bin/bash
# {id} — executable entry. Env: ANYCODE_SKILL_DIR, ANYCODE_WORKING_DIR.
set -euo pipefail
cd "${ANYCODE_WORKING_DIR:-.}"

echo "TODO: implement {id}; args: $*"
"#;

/// Create `<skills_root>/<id>/` with the standard skeleton. Refuses to
/// overwrite an existing skill directory.
pub fn scaffold_skill_dir(skills_root: &Path, id: &str, description: &str) -> Result<PathBuf> {
    if !SkillCatalog::is_valid_skill_id(id) {
        anyhow::bail!("invalid skill id {id:?} (letters, digits, . _ - only)");
    }
    let desc = description.trim();
    anyhow::ensure!(
        (30..=200).contains(&desc.chars().count()),
        "description must be 30–200 chars: state what it does, when to use, when not to"
    );
    let dir = skills_root.join(id);
    anyhow::ensure!(
        !dir.exists(),
        "skill directory {} already exists",
        dir.display()
    );
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    fs::write(
        dir.join("SKILL.md"),
        SKILL_MD_SKELETON
            .replace("{id}", id)
            .replace("{description}", desc),
    )
    .with_context(|| format!("write {}", dir.join("SKILL.md").display()))?;
    let run_path = dir.join("run");
    fs::write(&run_path, RUN_SKELETON.replace("{id}", id))
        .with_context(|| format!("write {}", run_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&run_path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("chmod +x {}", run_path.display()))?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaffold_produces_vet_clean_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = scaffold_skill_dir(
            tmp.path(),
            "demo-skill",
            "Demo skill that turns X into Y; use for demos, not for production reports.",
        )
        .unwrap();
        assert!(dir.join("SKILL.md").is_file());
        assert!(dir.join("run").is_file());
        // The skeleton must pass its own contract lint with zero findings.
        let report = super::super::vet::vet_skill_dir(&dir).unwrap();
        assert!(
            report.findings.is_empty(),
            "scaffold should be vet-clean: {:?}",
            report.findings
        );
        // And the manifest round-trips with the given description.
        let fm = super::super::parse_skill_manifest_file(&dir.join("SKILL.md")).unwrap();
        assert_eq!(fm.name, "demo-skill");
    }

    #[test]
    fn scaffold_rejects_existing_and_bad_input() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(scaffold_skill_dir(tmp.path(), "bad id!", "x".repeat(40).as_str()).is_err());
        assert!(scaffold_skill_dir(tmp.path(), "ok-id", "short").is_err());
        scaffold_skill_dir(
            tmp.path(),
            "ok-id",
            "A perfectly adequate description for the ok-id skill.",
        )
        .unwrap();
        assert!(scaffold_skill_dir(
            tmp.path(),
            "ok-id",
            "A perfectly adequate description for the ok-id skill.",
        )
        .is_err());
    }
}
