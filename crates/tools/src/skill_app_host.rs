//! Host-mediated Skill App present / brief wait (Workbench attaches an impl).

use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct SkillAppPresentRequest {
    pub skill_id: String,
    pub slot: String,
    pub wait_brief: bool,
    pub push: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct SkillAppPresentResponse {
    pub present_id: String,
    /// Set when `wait_brief` was true and the user submitted a VisualBrief.
    pub brief: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct SkillAppHostError(pub String);

impl std::fmt::Display for SkillAppHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for SkillAppHostError {}

#[async_trait]
pub trait SkillAppHost: Send + Sync {
    async fn present(
        &self,
        request: SkillAppPresentRequest,
    ) -> Result<SkillAppPresentResponse, SkillAppHostError>;

    async fn push(
        &self,
        skill_id: &str,
        slot: &str,
        payload: Value,
    ) -> Result<(), SkillAppHostError>;
}

pub type SkillAppHostArc = Arc<dyn SkillAppHost>;
