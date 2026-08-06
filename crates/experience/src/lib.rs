//! Offline experience distillation helpers (teacher lab tooling).
//!
//! Runtime only loads signed/validated packs; teacher API keys never ship in Desktop.

use anycode_core::{ExperienceCard, ExperiencePack, ExperiencePackMeta, TaskFamily};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

pub mod trajectory_extract;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeacherTrajectory {
    pub id: String,
    pub family: TaskFamily,
    pub prompt: String,
    #[serde(default)]
    pub tool_order: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub passed_gates: bool,
    #[serde(default)]
    pub low_model_replay_gain: f64,
}

/// Compress a successful teacher trajectory into an experience card candidate.
pub fn distill_card(traj: &TeacherTrajectory) -> ExperienceCard {
    ExperienceCard {
        id: format!("distill.{}", traj.id),
        title: format!("Distilled: {}", traj.id),
        family: traj.family,
        applicable_when: vec![traj.prompt.chars().take(80).collect()],
        task_breakdown: traj.notes.clone(),
        tool_order: traj.tool_order.clone(),
        key_checks: traj
            .notes
            .iter()
            .filter(|n| n.to_ascii_lowercase().contains("check"))
            .cloned()
            .collect(),
        common_failures: Vec::new(),
        recovery: vec!["replay failed gate with evidence".into()],
        examples: vec![traj.prompt.clone()],
        model_compat: vec!["weak_local".into()],
        regression_score: traj.low_model_replay_gain,
        version: "0.1.0".into(),
    }
}

/// Only keep trajectories that passed real gates and helped the low model.
pub fn filter_validated(trajs: &[TeacherTrajectory], min_gain: f64) -> Vec<&TeacherTrajectory> {
    trajs
        .iter()
        .filter(|t| t.passed_gates && t.low_model_replay_gain >= min_gain)
        .collect()
}

/// `sha256(secret ‖ payload)` 前 16 个 hex 字符 — 与 `scripts/compile-experience-pack.py`
/// 的 `sign()` 逐字节对齐（黄金向量见 tests）。
fn signature_hex_for(payload: &[u8], secret: &str) -> String {
    let mut h = Sha256::new();
    h.update(secret.as_bytes());
    h.update(payload);
    let hex = format!("{:x}", h.finalize());
    hex[..16].to_string()
}

pub fn sign_pack_hmac_like(pack: &mut ExperiencePack, secret: &str) {
    let payload = pack.signing_payload().unwrap_or_default();
    pack.meta.signature_hex = signature_hex_for(&payload, secret);
    pack.meta.signer = "offline-teacher-lab".into();
    if pack.meta.created_at.is_none() {
        pack.meta.created_at = Some(Utc::now());
    }
}

pub fn verify_pack_hmac_like(pack: &ExperiencePack, secret: &str) -> bool {
    if pack.meta.signature_hex.is_empty() {
        return false;
    }
    let payload = match pack.signing_payload() {
        Ok(p) => p,
        Err(_) => return false,
    };
    pack.meta.signature_hex == signature_hex_for(&payload, secret)
}

pub fn build_pack_from_trajectories(
    id: &str,
    version: &str,
    trajs: &[TeacherTrajectory],
    min_gain: f64,
) -> ExperiencePack {
    let cards = filter_validated(trajs, min_gain)
        .into_iter()
        .map(distill_card)
        .collect::<Vec<_>>();
    let regression_score = if cards.is_empty() {
        0.0
    } else {
        cards.iter().map(|c| c.regression_score).sum::<f64>() / cards.len() as f64
    };
    ExperiencePack {
        meta: ExperiencePackMeta {
            id: id.into(),
            version: version.into(),
            model_compat: vec!["weak_local".into(), "*".into()],
            regression_score,
            created_at: Some(Utc::now()),
            signature_hex: String::new(),
            signer: String::new(),
        },
        cards,
    }
}

pub fn load_pack(path: impl AsRef<Path>) -> anyhow::Result<ExperiencePack> {
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_pack(path: impl AsRef<Path>, pack: &ExperiencePack) -> anyhow::Result<()> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(pack)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anycode_core::TaskFamily;

    #[test]
    fn distill_sign_verify() {
        let trajs = vec![TeacherTrajectory {
            id: "web1".into(),
            family: TaskFamily::WebDesign,
            prompt: "dark landing page".into(),
            tool_order: vec!["Write".into(), "BrowserScreenshot".into()],
            notes: vec!["check contrast".into()],
            passed_gates: true,
            low_model_replay_gain: 0.3,
        }];
        let mut pack = build_pack_from_trajectories("lab", "0.1.0", &trajs, 0.1);
        assert_eq!(pack.cards.len(), 1);
        sign_pack_hmac_like(&mut pack, "test-secret");
        assert!(verify_pack_hmac_like(&pack, "test-secret"));
        assert!(!verify_pack_hmac_like(&pack, "other"));
    }

    /// 跨语言黄金向量：与 Python `sha256(secret+payload).hexdigest()[:16]` 对拍
    /// （python3 -c 'import hashlib; h=hashlib.sha256(); h.update(b"test-secret");
    /// h.update(b"{\"id\":\"lab\",\"version\":\"0.1.0\",\"cards\":[]}"); print(h.hexdigest()[:16])'）。
    #[test]
    fn golden_signature_vector_matches_python() {
        let mut pack = ExperiencePack {
            meta: ExperiencePackMeta {
                id: "lab".into(),
                version: "0.1.0".into(),
                model_compat: vec![],
                regression_score: 0.0,
                created_at: None,
                signature_hex: String::new(),
                signer: String::new(),
            },
            cards: vec![],
        };
        sign_pack_hmac_like(&mut pack, "test-secret");
        assert_eq!(pack.meta.signature_hex, "0422e0f29004e956");
    }
}
