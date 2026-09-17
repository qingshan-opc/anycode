//! Provider-usage fact envelope, NOT another wallet and NOT a fabricated billing API.
//! Send through a transactional outbox once 818cloud's metering endpoint is agreed.
use anycode_harness_core::{digest::digest, Error, Result, RunContext};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UsageReceipt {
    pub version: u32,
    pub idempotency_key: String,
    pub product: String,
    pub root_run_id: Uuid,
    pub run_id: Uuid,
    pub parent_run_id: Option<Uuid>,
    pub scope_digest: String,
    pub turn: u64,
    pub provider: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub measured: bool,
}
impl UsageReceipt {
    pub fn new(
        ctx: &RunContext,
        turn: u64,
        provider: &str,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        measured: bool,
    ) -> Result<Self> {
        if turn == 0
            || provider.is_empty()
            || model.is_empty()
            || provider.len() > 128
            || model.len() > 256
        {
            return Err(Error::Invalid("usage receipt metadata".into()));
        }
        let scope = ctx.scope().binding()?;
        let key = digest(&("anycode-usage-v1", &scope, ctx.id(), turn))?;
        Ok(Self {
            version: 1,
            idempotency_key: key,
            product: "anycode".into(),
            root_run_id: ctx.root_id(),
            run_id: ctx.id(),
            parent_run_id: ctx.parent_id(),
            scope_digest: scope,
            turn,
            provider: provider.into(),
            model: model.into(),
            input_tokens,
            output_tokens,
            measured,
        })
    }
    pub fn billable(&self) -> bool {
        self.measured
    }
}
