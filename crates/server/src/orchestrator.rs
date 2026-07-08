//! Agent 编排器：LLM 调用 → 工具调度 → 结果回填 → 再调用循环。
//!
//! 设计要点：
//! - [`Orchestrator`] 持有 `Arc<dyn LlmClient>` / `Arc<Sandbox>` 与 tool spec 清单，
//!   `PromptEngine` 用 `tokio::sync::Mutex` 包裹（其 `compile` 为 `&mut self`）。
//! - [`Orchestrator::run_once`] 为同步阻塞语义：跑完一轮或多轮工具循环后返回最终文本。
//! - [`Orchestrator::run_streaming`] 通过 `tokio::sync::mpsc` 桥接为事件流，供 WebSocket 用。
//! - [`Orchestrator::dispatch_subagent`] 用受限沙箱（只读工具）+ 角色 system prompt 隔离上下文。
//!
//! `Sandbox` 未暴露工具 schema，故 [`default_sandbox_with_specs`] / [`restricted_sandbox_with_specs`]
//! 在装配沙箱时同步抽出 `Vec<ToolSpec>`，由 Orchestrator 持有。

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use forgeclaw_core::model::{Section, ToolResult};
use forgeclaw_llm::{
    ChatMessage, ChatRequest, Event, FunctionCallDto, FunctionSpec, History, LlmClient,
    ToolCallDto, ToolSpec,
};
use forgeclaw_tools::{
    auto_confirm, FileReadTool, FileWriteTool, GrepTool, Sandbox, SearchTool, ShellTool, Tool,
};
use futures::StreamExt;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;
use tracing::warn;
use uuid::Uuid;

/// run_turn 默认最大轮数（SRV-004）。
const DEFAULT_MAX_TURNS: usize = 25;

/// 子代理角色。
#[derive(Debug, Clone, Copy)]
pub enum SubagentRole {
    /// 只读探索：浏览代码库并汇总结构。
    Explore,
    /// 深入研究：针对特定模块/问题深挖。
    Research,
    /// 代码审查：审查变更质量与风险。
    Review,
}

/// 编排器对外事件（流式 + 终态）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OrchestratorEvent {
    /// 文本增量。
    Delta { text: String },
    /// 工具调用开始（已聚合完整 input）。
    ToolCallStart { name: String, input: Value },
    /// 工具调用结果。
    ToolResult { name: String, result: ToolResult },
    /// 一轮对话完成（最终助手文本 + 本轮所有工具调用记录）。
    Complete {
        text: String,
        tool_calls: Vec<ToolCallRecord>,
    },
    /// 流内错误。
    Error { message: String },
}

/// 一次工具调用记录（用于 Complete 汇总）。
#[derive(Debug, Clone, Serialize)]
pub struct ToolCallRecord {
    pub id: String,
    pub name: String,
    pub result: ToolResult,
}

/// Agent 编排器。
pub struct Orchestrator {
    llm: Arc<dyn LlmClient>,
    sandbox: Arc<Sandbox>,
    tool_specs: Arc<Vec<ToolSpec>>,
    prompt_engine: tokio::sync::Mutex<forgeclaw_core::prompt::PromptEngine>,
    working_dir: PathBuf,
    model: String,
    profile: String,
}

impl Orchestrator {
    pub fn new(
        llm: Arc<dyn LlmClient>,
        sandbox: Arc<Sandbox>,
        tool_specs: Vec<ToolSpec>,
        prompts_root: PathBuf,
        profile: String,
        model: String,
        working_dir: PathBuf,
    ) -> Self {
        Self {
            llm,
            sandbox,
            tool_specs: Arc::new(tool_specs),
            prompt_engine: tokio::sync::Mutex::new(forgeclaw_core::prompt::PromptEngine::new(
                prompts_root,
            )),
            working_dir,
            model,
            profile,
        }
    }

    /// 工具规格清单（REST /api/tools 用）。
    pub fn tool_specs(&self) -> &[ToolSpec] {
        &self.tool_specs
    }

    /// 工作目录。
    pub fn working_dir(&self) -> &std::path::Path {
        &self.working_dir
    }

    /// 编译 system prompt（注入 tools/model/cwd 变量）。
    pub async fn compile_prompt(&self, profile: &str) -> anyhow::Result<String> {
        let mut engine = self.prompt_engine.lock().await;
        let vars = self.prompt_vars();
        let prompt = engine.compile(profile, &vars)?;
        Ok(prompt)
    }

    /// 列出 profile 启用的 sections。
    pub async fn list_sections(&self, profile: &str) -> anyhow::Result<Vec<Section>> {
        let engine = self.prompt_engine.lock().await;
        let sections = engine.list_sections(profile)?;
        Ok(sections)
    }

    fn prompt_vars(&self) -> HashMap<&'static str, String> {
        let tools_list = self
            .tool_specs
            .iter()
            .map(|s| s.function.name.clone())
            .collect::<Vec<_>>()
            .join(", ");
        let mut vars: HashMap<&str, String> = HashMap::new();
        vars.insert("tools", tools_list);
        vars.insert("model", self.model.clone());
        vars.insert("cwd", self.working_dir.display().to_string());
        vars
    }

    async fn compile_system_prompt(&self) -> anyhow::Result<String> {
        let mut engine = self.prompt_engine.lock().await;
        let vars = self.prompt_vars();
        let prompt = engine.compile(&self.profile, &vars)?;
        Ok(prompt)
    }

    /// 一次性运行：必要时注入 system prompt，跑完工具循环后返回最终事件。
    pub async fn run_once(
        &self,
        history: &mut History,
        user_msg: String,
    ) -> anyhow::Result<OrchestratorEvent> {
        if history.is_empty() {
            let prompt = self.compile_system_prompt().await?;
            *history = History::with_system(prompt);
        }
        self.run_turn(
            self.sandbox.as_ref(),
            &self.tool_specs,
            history,
            user_msg,
            None,
            DEFAULT_MAX_TURNS,
        )
        .await
    }

    /// 流式运行：把事件推入 `tx`，直到 Complete/Error。
    /// 不内部 spawn；调用方应将其置于独立任务并发排空 receiver。
    pub async fn run_streaming(
        &self,
        history: &mut History,
        user_msg: String,
        tx: mpsc::Sender<OrchestratorEvent>,
    ) -> anyhow::Result<()> {
        if history.is_empty() {
            let prompt = self.compile_system_prompt().await?;
            *history = History::with_system(prompt);
        }
        self.run_turn(
            self.sandbox.as_ref(),
            &self.tool_specs,
            history,
            user_msg,
            Some(&tx),
            DEFAULT_MAX_TURNS,
        )
        .await?;
        Ok(())
    }

    /// 派发子代理：受限沙箱（只读工具）+ 角色 system prompt，跑最多 5 轮，
    /// 仅返回最终 summary 文本（隔离上下文，省 token）。
    pub async fn dispatch_subagent(
        &self,
        role: SubagentRole,
        task: String,
    ) -> anyhow::Result<String> {
        let (sandbox, tool_specs) = restricted_sandbox_with_specs(self.working_dir.clone());
        let sandbox = Arc::new(sandbox);
        let system_prompt = subagent_system_prompt(role);
        let mut history = History::with_system(system_prompt);
        let mut msg = task;
        for _round in 0..5 {
            match self
                .run_turn(
                    &sandbox,
                    &tool_specs,
                    &mut history,
                    std::mem::take(&mut msg),
                    None,
                    DEFAULT_MAX_TURNS,
                )
                .await
            {
                Ok(OrchestratorEvent::Complete { text, .. }) => return Ok(text),
                Ok(_) => continue,
                Err(e) => return Err(e),
            }
        }
        Ok("subagent: rounds exhausted".to_string())
    }

    /// 核心单轮循环：append user → 反复 (LLM 调用 → 工具执行 → 回填) 直到无工具调用。
    ///
    /// `tx` 为 `Some` 时把流式事件推入 channel。`sandbox` / `tool_specs` 可被
    /// 覆盖（子代理用受限集合）。`max_turns` 限制 LLM 调用轮数（SRV-004）。
    ///
    /// 行为约定：
    /// - `Event::Error` 立即返回 `OrchestratorEvent::Error`，不构造 assistant_msg、不 append（SRV-005）。
    /// - `tx.send` 失败（客户端断开）立即返回 Error 停止生成（SRV-012）。
    /// - `parse_tool_input` 失败时构造错误 ToolResult 回填，不执行工具（SRV-019）。
    /// - 空 `tool_call_id` 自动生成 `call_<uuid>`（SRV-020）。
    async fn run_turn(
        &self,
        sandbox: &Sandbox,
        tool_specs: &[ToolSpec],
        history: &mut History,
        user_msg: String,
        tx: Option<&mpsc::Sender<OrchestratorEvent>>,
        max_turns: usize,
    ) -> anyhow::Result<OrchestratorEvent> {
        history.append(ChatMessage::user(user_msg));
        let mut records: Vec<ToolCallRecord> = Vec::new();

        let mut turn = 0usize;
        loop {
            turn += 1;
            if turn > max_turns {
                let event = OrchestratorEvent::Error {
                    message: "max turns exceeded".into(),
                };
                if let Some(tx) = tx {
                    let _ = tx.send(event.clone()).await;
                }
                return Ok(event);
            }

            let req = ChatRequest::from_history(
                history,
                &self.model,
                Some(0.0),
                None,
                Some(tool_specs.to_vec()),
            );
            let mut stream = self.llm.chat(req).await?;
            let mut text = String::new();
            let mut tcs: BTreeMap<u32, ToolCallAgg> = BTreeMap::new();

            while let Some(event) = stream.next().await {
                match event {
                    Event::Delta(s) => {
                        text.push_str(&s);
                        if let Some(tx) = tx {
                            if tx.send(OrchestratorEvent::Delta { text: s }).await.is_err() {
                                return Ok(OrchestratorEvent::Error {
                                    message: "client disconnected".into(),
                                });
                            }
                        }
                    }
                    Event::ToolCallDelta {
                        index,
                        id,
                        name,
                        arguments,
                    } => {
                        let entry = tcs.entry(index).or_default();
                        if let Some(id) = id {
                            entry.id = id;
                        }
                        if let Some(n) = name {
                            entry.name = n;
                        }
                        if let Some(a) = arguments {
                            entry.arguments.push_str(&a);
                        }
                    }
                    Event::Done => {}
                    // SRV-005：流内错误立即返回 Error，不构造 assistant_msg、不 append。
                    Event::Error(message) => {
                        warn!(%message, "llm stream error event");
                        let event = OrchestratorEvent::Error { message };
                        if let Some(tx) = tx {
                            let _ = tx.send(event.clone()).await;
                        }
                        return Ok(event);
                    }
                }
            }

            // SRV-020：空 tool_call_id 自动补全，保证 assistant_msg 与 tool_msg 引用一致。
            for agg in tcs.values_mut() {
                if agg.id.is_empty() {
                    agg.id = format!("call_{}", Uuid::new_v4());
                }
            }

            let assistant_msg = if tcs.is_empty() {
                ChatMessage::assistant(&text)
            } else {
                let dtos: Vec<ToolCallDto> = tcs.values().map(ToolCallDto::from).collect();
                ChatMessage {
                    role: "assistant".into(),
                    content: text.clone(),
                    tool_calls: Some(dtos),
                    tool_call_id: None,
                }
            };
            history.append(assistant_msg);

            if tcs.is_empty() {
                let event = OrchestratorEvent::Complete {
                    text,
                    tool_calls: records,
                };
                if let Some(tx) = tx {
                    if tx.send(event.clone()).await.is_err() {
                        return Ok(OrchestratorEvent::Error {
                            message: "client disconnected".into(),
                        });
                    }
                }
                return Ok(event);
            }

            for agg in tcs.into_values() {
                let parsed = parse_tool_input(&agg.arguments);
                // SRV-019：parse 失败时构造错误 ToolResult 回填，不执行工具。
                let result = match parsed {
                    Ok(v) => {
                        if let Some(tx) = tx {
                            if tx
                                .send(OrchestratorEvent::ToolCallStart {
                                    name: agg.name.clone(),
                                    input: v.clone(),
                                })
                                .await
                                .is_err()
                            {
                                return Ok(OrchestratorEvent::Error {
                                    message: "client disconnected".into(),
                                });
                            }
                        }
                        match sandbox.execute(&agg.name, v).await {
                            Ok(r) => r,
                            Err(e) => ToolResult {
                                output: String::new(),
                                error: Some(e.to_string()),
                                duration_ms: 0,
                            },
                        }
                    }
                    Err(e) => {
                        let err_msg = format!("invalid tool input: {e}");
                        if let Some(tx) = tx {
                            if tx
                                .send(OrchestratorEvent::ToolCallStart {
                                    name: agg.name.clone(),
                                    input: Value::Null,
                                })
                                .await
                                .is_err()
                            {
                                return Ok(OrchestratorEvent::Error {
                                    message: "client disconnected".into(),
                                });
                            }
                        }
                        ToolResult {
                            output: String::new(),
                            error: Some(err_msg),
                            duration_ms: 0,
                        }
                    }
                };
                if let Some(tx) = tx {
                    if tx
                        .send(OrchestratorEvent::ToolResult {
                            name: agg.name.clone(),
                            result: result.clone(),
                        })
                        .await
                        .is_err()
                    {
                        return Ok(OrchestratorEvent::Error {
                            message: "client disconnected".into(),
                        });
                    }
                }
                records.push(ToolCallRecord {
                    id: agg.id.clone(),
                    name: agg.name.clone(),
                    result: result.clone(),
                });
                let tool_msg = ChatMessage {
                    role: "tool".into(),
                    content: result.output,
                    tool_calls: None,
                    tool_call_id: Some(agg.id),
                };
                history.append(tool_msg);
            }
        }
    }
}

/// 工具调用聚合（按 index 累积 id/name/arguments 字符串）。
#[derive(Default)]
struct ToolCallAgg {
    id: String,
    name: String,
    arguments: String,
}

impl From<&ToolCallAgg> for ToolCallDto {
    fn from(agg: &ToolCallAgg) -> Self {
        ToolCallDto {
            id: agg.id.clone(),
            function: FunctionCallDto {
                name: agg.name.clone(),
                arguments: agg.arguments.clone(),
            },
        }
    }
}

/// 解析工具调用 arguments JSON 字符串。空串视为空对象。
/// 解析失败返回 `Err`，由 [`Orchestrator::run_turn`] 构造错误 ToolResult 回填（SRV-019）。
fn parse_tool_input(args: &str) -> Result<Value, serde_json::Error> {
    if args.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    serde_json::from_str(args)
}

fn subagent_system_prompt(role: SubagentRole) -> String {
    match role {
        SubagentRole::Explore => concat!(
            "你是只读探索子代理。使用 read/search/grep 浏览代码库，",
            "汇总目录结构与关键文件，不修改任何文件、不执行 shell。最终给出简明探索摘要。"
        ),
        SubagentRole::Research => concat!(
            "你是深入研究子代理。使用 read/search/grep 针对指定问题深挖，",
            "不修改任何文件、不执行 shell。最终给出聚焦的研究结论与证据引用。"
        ),
        SubagentRole::Review => concat!(
            "你是代码审查子代理。使用 read/search/grep 检视相关代码，",
            "不修改任何文件、不执行 shell。最终给出风险、缺陷与改进建议清单。"
        ),
    }
    .to_string()
}

fn spec_for(tool: &dyn Tool, description: &str) -> ToolSpec {
    ToolSpec {
        typ: "function".into(),
        function: FunctionSpec {
            name: tool.name().to_string(),
            description: description.to_string(),
            parameters: tool.schema(),
        },
    }
}

/// 装配默认沙箱（5 工具：shell/read/write/search/grep）+ 对应 ToolSpec 清单。
/// 因 `Sandbox` 不暴露工具 schema，须在注册前从工具实例抽出。
pub fn default_sandbox_with_specs(working_dir: PathBuf) -> (Sandbox, Vec<ToolSpec>) {
    let shell = ShellTool::new(working_dir.clone());
    let read = FileReadTool::new(working_dir.clone());
    let write = FileWriteTool::new(working_dir.clone());
    let search = SearchTool::new(working_dir.clone());
    let grep = GrepTool::new(working_dir.clone());

    let specs = vec![
        spec_for(&shell, "Execute shell commands in the working directory"),
        spec_for(&read, "Read a file from the working directory"),
        spec_for(&write, "Write content to a file in the working directory"),
        spec_for(
            &search,
            "Search files by glob pattern in the working directory",
        ),
        spec_for(
            &grep,
            "Grep file contents by regex in the working directory",
        ),
    ];

    let mut sb = Sandbox::new(working_dir, auto_confirm());
    sb.register(Box::new(shell));
    sb.register(Box::new(read));
    sb.register(Box::new(write));
    sb.register(Box::new(search));
    sb.register(Box::new(grep));
    (sb, specs)
}

/// 装配受限沙箱（只读 3 工具：read/search/grep）+ 对应 ToolSpec 清单。
/// 供子代理使用，确保不写不执行 shell。
pub fn restricted_sandbox_with_specs(working_dir: PathBuf) -> (Sandbox, Vec<ToolSpec>) {
    let read = FileReadTool::new(working_dir.clone());
    let search = SearchTool::new(working_dir.clone());
    let grep = GrepTool::new(working_dir.clone());

    let specs = vec![
        spec_for(&read, "Read a file from the working directory"),
        spec_for(
            &search,
            "Search files by glob pattern in the working directory",
        ),
        spec_for(
            &grep,
            "Grep file contents by regex in the working directory",
        ),
    ];

    let mut sb = Sandbox::new(working_dir, auto_confirm());
    sb.register(Box::new(read));
    sb.register(Box::new(search));
    sb.register(Box::new(grep));
    (sb, specs)
}
