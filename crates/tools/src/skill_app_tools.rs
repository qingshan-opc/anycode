//! SkillAppPresent / SkillAppPush / SkillAppRead tools.

use crate::services::ToolServices;
use crate::skill_app_host::{SkillAppHostError, SkillAppPresentRequest};
use crate::skills::{load_skill_surface, SkillCatalog};
use anycode_core::prelude::*;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

fn skill_root_for(
    services: &ToolServices,
    skill_id: &str,
    task_cwd: Option<&std::path::Path>,
) -> Option<PathBuf> {
    services
        .skill_catalog
        .resolve_skill_root(skill_id, task_cwd)
}

#[derive(Debug, Deserialize)]
struct PresentIn {
    skill_id: String,
    #[serde(default)]
    slot: Option<String>,
    #[serde(default)]
    wait: Option<String>,
    #[serde(default)]
    push: Option<Value>,
}

pub struct SkillAppPresentTool {
    services: Arc<ToolServices>,
    policy: SecurityPolicy,
}

impl SkillAppPresentTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self {
            services,
            policy: SecurityPolicy {
                require_approval: false,
                ..SecurityPolicy::default()
            },
        }
    }
}

#[async_trait]
impl Tool for SkillAppPresentTool {
    fn name(&self) -> &str {
        "SkillAppPresent"
    }

    fn description(&self) -> &str {
        "Open a skill's visual Skill App in the Workbench so the user can pick style/templates. \
         For PPT (`anycode-ppt`) or short video (`anycode-video`) when the user message has no \
         `[Host VisualBrief …]` yet: call with wait=\"brief\", then follow the returned brief. \
         Do NOT call again when a VisualBrief is already in the user message or tool result. \
         Do not guess themes from keywords — let the user pick in the studio."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": {
                    "type": "string",
                    "description": "Skill directory id that ships ui/ (e.g. anycode-ppt)."
                },
                "slot": {
                    "type": "string",
                    "enum": ["dock", "conversation", "project"],
                    "description": "Host slot. Default: skill surface default_slot or dock."
                },
                "wait": {
                    "type": "string",
                    "enum": ["brief", "none"],
                    "description": "Wait for VisualBrief submit (brief) or just open (none)."
                },
                "push": {
                    "type": "object",
                    "description": "Optional initial payload for the mini-app."
                }
            },
            "required": ["skill_id"]
        })
    }

    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Default
    }

    fn security_policy(&self) -> Option<&SecurityPolicy> {
        Some(&self.policy)
    }

    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, CoreError> {
        let start = Instant::now();
        let args: PresentIn =
            serde_json::from_value(input.input).map_err(CoreError::SerializationError)?;
        let skill_id = args.skill_id.trim().to_string();
        if skill_id.is_empty() || !SkillCatalog::is_valid_skill_id(&skill_id) {
            return Ok(ToolOutput {
                result: json!({"error": "invalid skill_id"}),
                error: Some("invalid skill_id".into()),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        }
        let task_cwd = input.working_directory.as_deref().map(std::path::Path::new);
        let Some(root) = skill_root_for(&self.services, &skill_id, task_cwd) else {
            return Ok(ToolOutput {
                result: json!({"error": format!("skill not found: {skill_id}")}),
                error: Some("skill not found".into()),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        };
        let surface = load_skill_surface(&root);
        if surface.is_none() {
            return Ok(ToolOutput {
                result: json!({"error": format!("skill {skill_id} has no ui/ Skill App")}),
                error: Some("no ui".into()),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        }
        let default_slot = surface
            .as_ref()
            .map(|s| s.default_slot.as_str().to_string())
            .unwrap_or_else(|| "dock".into());
        let slot = args.slot.unwrap_or(default_slot);
        let wait_brief = matches!(args.wait.as_deref(), Some("brief"));

        let Some(host) = self.services.skill_app_host() else {
            return Ok(ToolOutput {
                result: json!({
                    "error": "No Skill App host (Workbench required)",
                    "unsupported_host": true
                }),
                error: Some("unsupported_host".into()),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        };

        match host
            .present(SkillAppPresentRequest {
                skill_id: skill_id.clone(),
                slot: slot.clone(),
                wait_brief,
                push: args.push,
            })
            .await
        {
            Ok(resp) => Ok(ToolOutput {
                result: json!({
                    "ok": true,
                    "present_id": resp.present_id,
                    "skill_id": skill_id,
                    "slot": slot,
                    "wait_brief": wait_brief,
                    "brief": resp.brief,
                }),
                error: None,
                duration_ms: start.elapsed().as_millis() as u64,
            }),
            Err(SkillAppHostError(msg)) => Ok(ToolOutput {
                result: json!({"error": msg}),
                error: Some(msg),
                duration_ms: start.elapsed().as_millis() as u64,
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
struct PushIn {
    skill_id: String,
    #[serde(default)]
    slot: Option<String>,
    payload: Value,
}

pub struct SkillAppPushTool {
    services: Arc<ToolServices>,
}

impl SkillAppPushTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

#[async_trait]
impl Tool for SkillAppPushTool {
    fn name(&self) -> &str {
        "SkillAppPush"
    }

    fn description(&self) -> &str {
        "Non-blocking push of preview/progress data into an open Skill App mini-app."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": { "type": "string" },
                "slot": {
                    "type": "string",
                    "enum": ["dock", "conversation", "project"]
                },
                "payload": {
                    "type": "object",
                    "description": "JSON payload delivered to anycode.agent.onPush."
                }
            },
            "required": ["skill_id", "payload"]
        })
    }

    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Default
    }

    fn security_policy(&self) -> Option<&SecurityPolicy> {
        None
    }

    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, CoreError> {
        let start = Instant::now();
        let args: PushIn =
            serde_json::from_value(input.input).map_err(CoreError::SerializationError)?;
        let skill_id = args.skill_id.trim().to_string();
        if skill_id.is_empty() {
            return Ok(ToolOutput {
                result: json!({"error": "skill_id required"}),
                error: Some("skill_id required".into()),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        }
        let slot = args.slot.unwrap_or_else(|| "dock".into());
        let Some(host) = self.services.skill_app_host() else {
            return Ok(ToolOutput {
                result: json!({"error": "No Skill App host", "unsupported_host": true}),
                error: Some("unsupported_host".into()),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        };
        match host.push(&skill_id, &slot, args.payload).await {
            Ok(()) => Ok(ToolOutput {
                result: json!({"ok": true, "skill_id": skill_id, "slot": slot}),
                error: None,
                duration_ms: start.elapsed().as_millis() as u64,
            }),
            Err(SkillAppHostError(msg)) => Ok(ToolOutput {
                result: json!({"error": msg}),
                error: Some(msg),
                duration_ms: start.elapsed().as_millis() as u64,
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReadIn {
    skill_id: String,
    #[serde(default)]
    project_id: Option<String>,
}

pub struct SkillAppReadTool {
    services: Arc<ToolServices>,
}

impl SkillAppReadTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

#[async_trait]
impl Tool for SkillAppReadTool {
    fn name(&self) -> &str {
        "SkillAppRead"
    }

    fn description(&self) -> &str {
        "Read the current VisualBrief and Skill App state for a skill in the active project. \
         Call this before generating when a brief may already be locked."
    }

    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill_id": { "type": "string" },
                "project_id": {
                    "type": "string",
                    "description": "Optional; defaults to ANYCODE_DASHBOARD_PROJECT_ID when set."
                }
            },
            "required": ["skill_id"]
        })
    }

    fn permission_mode(&self) -> PermissionMode {
        PermissionMode::Default
    }

    fn security_policy(&self) -> Option<&SecurityPolicy> {
        None
    }

    async fn execute(&self, input: ToolInput) -> Result<ToolOutput, CoreError> {
        let start = Instant::now();
        let args: ReadIn =
            serde_json::from_value(input.input).map_err(CoreError::SerializationError)?;
        let skill_id = args.skill_id.trim().to_string();
        if skill_id.is_empty() {
            return Ok(ToolOutput {
                result: json!({"error": "skill_id required"}),
                error: Some("skill_id required".into()),
                duration_ms: start.elapsed().as_millis() as u64,
            });
        }
        let _ = &self.services;
        let project_id = args
            .project_id
            .or_else(|| std::env::var("ANYCODE_DASHBOARD_PROJECT_ID").ok())
            .unwrap_or_default();
        let state_path = dirs::home_dir().map(|h| {
            h.join(".anycode/dashboard/skill-apps/state")
                .join(&project_id)
                .join(format!("{skill_id}.json"))
        });
        let (state, brief) = if let Some(path) = state_path.as_ref().filter(|p| p.is_file()) {
            let raw = std::fs::read_to_string(path).unwrap_or_default();
            let v: Value = serde_json::from_str(&raw).unwrap_or(json!({}));
            let brief = v.get("brief").cloned();
            let state = v.get("state").cloned().unwrap_or(json!({}));
            (state, brief)
        } else {
            (json!({}), None)
        };
        Ok(ToolOutput {
            result: json!({
                "ok": true,
                "skill_id": skill_id,
                "project_id": project_id,
                "brief": brief,
                "state": state,
                "has_brief": brief.is_some(),
            }),
            error: None,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }
}
