//! Feature-gated migration seam. Existing Workbench and scheduler routes are NOT
//! switched automatically. New code still calls the real legacy security pipeline.
use super::AgentRuntime;
use anycode_core::{AgentType,Message,ModelConfig,ToolCall};
use anycode_harness_core::{events::PreviewBus,types::{Invocation,ToolResult,ToolSpec},Error,Result,RunContext};
use anycode_harness_host::{AnyCodeHost,CheckedExecutor};
use async_trait::async_trait;
use std::{collections::BTreeMap,path::{Path,PathBuf},sync::Arc};

#[async_trait]
pub trait HarnessBoundary:Send+Sync{
    /// Mandatory live project/tenant authorization + full input validation + path /
    /// device / approval checks. This executes IN ADDITION TO SecurityLayer.
    async fn preflight(&self,ctx:&RunContext,call:&Invocation)->Result<()>;
    async fn transform(&self,ctx:&RunContext,history:&[Message])->Result<Vec<Message>>;
    async fn completion(&self,ctx:&RunContext,history:&[Message])->Result<()>;
}
struct RuntimeExecutor{
    runtime:Arc<AgentRuntime>,working_directory:PathBuf,agent:AgentType,
    boundary:Arc<dyn HarnessBoundary>,root_run:uuid::Uuid,scope_digest:String,
    allowed:BTreeMap<String,(String,bool)>,
}
#[async_trait]impl CheckedExecutor for RuntimeExecutor{
    async fn invoke(&self,ctx:&RunContext,call:&Invocation)->Result<ToolResult>{
        ctx.check()?;
        if ctx.scope().binding()?!=self.scope_digest||ctx.root_id()!=self.root_run{return Err(Error::Denied("host scope/root mismatch".into()))}
        let(capability,read_only)=self.allowed.get(&call.name).ok_or_else(||Error::Denied("tool outside host registry".into()))?;
        ctx.capabilities().require(capability)?;
        self.boundary.preflight(ctx,call).await?;ctx.check()?;
        let input=ToolCall{id:call.id.clone(),name:call.name.clone(),input:call.arguments.clone()};
        let out=self.runtime.execute_tool_call(ctx.id(),&self.agent,&self.working_directory.to_string_lossy(),&input).await;
        match out{
            Ok(out)=>Ok(ToolResult{value:out.result,is_error:out.error.is_some()}),
            Err(_) if !*read_only=>Err(Error::Uncertain("legacy mutating tool returned an error; inspect effects".into())),
            Err(_)=>Err(Error::Host("legacy read-only tool failed".into())),
        }
    }
    async fn transform(&self,ctx:&RunContext,history:&[Message])->Result<Vec<Message>>{self.boundary.transform(ctx,history).await}
    async fn completion(&self,ctx:&RunContext,history:&[Message])->Result<()>{self.boundary.completion(ctx,history).await}
}
impl AgentRuntime{
    /// Build a host from trusted allowlisted tool bindings. This does not register
    /// a second tool registry or bypass the current runtime's tool gating.
    pub async fn harness_host(self:&Arc<Self>,ctx:&RunContext,working_directory:&Path,agent:AgentType,
        model:ModelConfig,bindings:BTreeMap<String,(String,bool)>,boundary:Arc<dyn HarnessBoundary>,previews:PreviewBus)->Result<AnyCodeHost>{
        ctx.check()?;
        if !self.sandbox_mode{return Err(Error::Denied("harness pilot requires sandbox_mode".into()))}
        if bindings.is_empty()||bindings.len()>128{return Err(Error::Invalid("harness binding count".into()))}
        let wd=std::fs::canonicalize(working_directory)?;
        // Prevent an old nested Agent tool from escaping into the legacy orchestration
        // loop. New subagents must use Supervisor + the same Kernel and RunContext.
        let forbidden=["Agent","Task","CronCreate","RemoteTrigger","ScheduleWakeup","Config","PowerShell","Repl"];
        if bindings.keys().any(|n|forbidden.contains(&n.as_str())){return Err(Error::Denied("legacy orchestration/control tools are not allowed in harness pilot".into()))}
        let tools=self.tools.read().await;let mut registry=vec![];
        for(name,(capability,read_only))in &bindings{
            ctx.capabilities().require(capability)?;
            let tool=tools.get(name).ok_or_else(||Error::Invalid(format!("unknown registered tool: {name}")))?;
            registry.push(ToolSpec{name:name.clone(),description:tool.api_tool_description(),input_schema:tool.schema(),capability:capability.clone(),read_only:*read_only});
        }
        drop(tools);
        Ok(AnyCodeHost{llm:self.llm_client.clone(),model,registry,streaming:true,previews,
            executor:Arc::new(RuntimeExecutor{runtime:self.clone(),working_directory:wd,agent,boundary,root_run:ctx.root_id(),scope_digest:ctx.scope().binding()?,allowed:bindings})})
    }
}
/// Conservative local pilot: ONLY FileRead in an immutable host-provisioned root.
/// Not suitable as a multi-tenant authorization implementation. No implicit cloud grant.
pub struct ReadOnlyPilotBoundary{scope:String,root:PathBuf}
impl ReadOnlyPilotBoundary{
    pub fn new(ctx:&RunContext,root:&Path)->Result<Self>{
        if ctx.scope().tenant.is_some(){return Err(Error::Denied("enterprise requires live ProductAcl boundary".into()))}
        Ok(Self{scope:ctx.scope().binding()?,root:std::fs::canonicalize(root)?})
    }
}
#[derive(serde::Deserialize)]#[serde(deny_unknown_fields)]struct ReadArgs{file_path:String,#[serde(default)]offset:Option<usize>,#[serde(default)]limit:Option<usize>}
#[async_trait]impl HarnessBoundary for ReadOnlyPilotBoundary{
    async fn preflight(&self,ctx:&RunContext,call:&Invocation)->Result<()>{
        ctx.check()?;if ctx.scope().binding()?!=self.scope||call.name!="FileRead"{return Err(Error::Denied("pilot permits only scoped FileRead".into()))}
        let args:ReadArgs=serde_json::from_value(call.arguments.clone()).map_err(|_|Error::Invalid("FileRead schema".into()))?;
        if args.offset==Some(0)||args.limit==Some(0)||args.limit.is_some_and(|n|n>10000){return Err(Error::Invalid("read line bounds".into()))}
        let p=Path::new(&args.file_path);let p=std::fs::canonicalize(if p.is_absolute(){p.to_path_buf()}else{self.root.join(p)})?;
        if !p.starts_with(&self.root)||!p.is_file()||std::fs::metadata(&p)?.len()>4*1024*1024{return Err(Error::Denied("read path/size outside pilot limits".into()))}Ok(())
    }
    async fn transform(&self,ctx:&RunContext,history:&[Message])->Result<Vec<Message>>{
        ctx.check()?;if ctx.scope().binding()?!=self.scope{return Err(Error::Denied("pilot scope".into()))}Ok(history.to_vec())
    }
    async fn completion(&self,ctx:&RunContext,_history:&[Message])->Result<()>{ctx.check()}
}
