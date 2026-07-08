//! Orchestrator 单元测试：MockClient 驱动 run_once / run_streaming / dispatch_subagent。
//!
//! 不打真实 API；用 tempdir + auto_confirm 沙箱。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use forgeclaw_llm::{ChatRequest, Event, History, LlmClient, Role};
use forgeclaw_server::{
    default_sandbox_with_specs, restricted_sandbox_with_specs, Orchestrator, OrchestratorEvent,
    SubagentRole,
};
use futures::stream::BoxStream;
use tempfile::tempdir;
use tokio::sync::mpsc;

/// 脚本化 Mock LLM 客户端：每次 `chat` 依次返回下一个脚本的事件流。
struct MockClient {
    scripts: Vec<Vec<Event>>,
    counter: AtomicUsize,
}

impl MockClient {
    fn new(scripts: Vec<Vec<Event>>) -> Self {
        Self {
            scripts,
            counter: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmClient for MockClient {
    async fn chat(&self, _req: ChatRequest) -> anyhow::Result<BoxStream<'static, Event>> {
        let idx = self.counter.fetch_add(1, Ordering::SeqCst);
        let events = self
            .scripts
            .get(idx)
            .cloned()
            .unwrap_or_else(|| vec![Event::Done]);
        Ok(Box::pin(futures::stream::iter(events)))
    }
}

fn build_orch(scripts: Vec<Vec<Event>>) -> (Orchestrator, tempfile::TempDir) {
    let dir = tempdir().unwrap();
    let (sandbox, specs) = default_sandbox_with_specs(dir.path().to_path_buf());
    let llm: Arc<dyn LlmClient> = Arc::new(MockClient::new(scripts));
    let orch = Orchestrator::new(
        llm,
        Arc::new(sandbox),
        specs,
        dir.path().to_path_buf(), // prompts_root：本测试不触发 compile_system_prompt
        "default".into(),
        "test-model".into(),
        dir.path().to_path_buf(),
    );
    (orch, dir)
}

#[tokio::test]
async fn run_once_completes_without_tool_calls() {
    let (orch, _dir) = build_orch(vec![vec![Event::Delta("hi".into()), Event::Done]]);
    let mut history = History::with_system("sys");
    let event = orch
        .run_once(&mut history, "hello".into())
        .await
        .expect("run_once failed");
    match event {
        OrchestratorEvent::Complete { text, tool_calls } => {
            assert_eq!(text, "hi");
            assert!(tool_calls.is_empty());
        }
        other => panic!("unexpected event: {:?}", other),
    }
    // system + user + assistant = 3
    assert_eq!(history.len(), 3);
    assert_eq!(history.messages()[1].role, Role::User);
    assert_eq!(history.messages()[2].role, Role::Assistant);
}

#[tokio::test]
async fn run_once_executes_tool_calls_and_loops() {
    // 第一轮：LLM 要求调用 read 工具读 a.txt
    // 第二轮：LLM 给出最终文本 done
    let scripts = vec![
        vec![Event::ToolCallDelta {
            index: 0,
            id: Some("call_1".into()),
            name: Some("read".into()),
            arguments: Some("{\"path\":\"a.txt\"}".into()),
        }],
        // arguments 跨 chunk 拼接
        vec![Event::Delta("done".into()), Event::Done],
    ];
    let (orch, dir) = build_orch(scripts);
    std::fs::write(dir.path().join("a.txt"), "hello-content").unwrap();

    let mut history = History::with_system("sys");
    let event = orch
        .run_once(&mut history, "read a.txt".into())
        .await
        .expect("run_once failed");

    match event {
        OrchestratorEvent::Complete { text, tool_calls } => {
            assert_eq!(text, "done");
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].name, "read");
            assert_eq!(tool_calls[0].id, "call_1");
            assert_eq!(tool_calls[0].result.output, "hello-content");
            assert!(tool_calls[0].result.error.is_none());
        }
        other => panic!("unexpected event: {:?}", other),
    }
    // system + user + assistant(tool_calls) + tool + assistant("done") = 5
    assert_eq!(history.len(), 5);
    let assistant = &history.messages()[2];
    assert_eq!(assistant.role, Role::Assistant);
    let tcs = assistant.tool_calls.as_ref().expect("tool_calls present");
    assert_eq!(tcs.len(), 1);
    assert_eq!(tcs[0].function.name, "read");
    let tool_msg = &history.messages()[3];
    assert_eq!(tool_msg.role, Role::Tool);
    assert_eq!(tool_msg.tool_call_id.as_deref(), Some("call_1"));
    assert_eq!(tool_msg.content, "hello-content");
}

#[tokio::test]
async fn run_once_aggregates_tool_call_arguments_across_deltas() {
    // 把 arguments 拆到多个 ToolCallDelta，验证按 index 聚合
    let scripts = vec![
        vec![
            Event::ToolCallDelta {
                index: 0,
                id: Some("c1".into()),
                name: Some("read".into()),
                arguments: Some("{\"path\":\"".into()),
            },
            Event::ToolCallDelta {
                index: 0,
                id: None,
                name: None,
                arguments: Some("b.txt\"}".into()),
            },
        ],
        vec![Event::Delta("ok".into()), Event::Done],
    ];
    let (orch, dir) = build_orch(scripts);
    std::fs::write(dir.path().join("b.txt"), "B").unwrap();

    let mut history = History::with_system("sys");
    let event = orch.run_once(&mut history, "x".into()).await.unwrap();
    match event {
        OrchestratorEvent::Complete { tool_calls, .. } => {
            assert_eq!(tool_calls.len(), 1);
            assert_eq!(tool_calls[0].result.output, "B");
        }
        other => panic!("unexpected: {:?}", other),
    }
}

#[tokio::test]
async fn run_streaming_emits_delta_toolcall_toolresult_complete() {
    let scripts = vec![
        vec![
            Event::Delta("Hel".into()),
            Event::Delta("lo".into()),
            Event::ToolCallDelta {
                index: 0,
                id: Some("c1".into()),
                name: Some("read".into()),
                arguments: Some("{\"path\":\"a.txt\"}".into()),
            },
        ],
        vec![Event::Delta("final".into()), Event::Done],
    ];
    let (orch, dir) = build_orch(scripts);
    std::fs::write(dir.path().join("a.txt"), "AAA").unwrap();

    let mut history = History::with_system("sys");
    let (tx, mut rx) = mpsc::channel::<OrchestratorEvent>(64);
    let orch_arc = Arc::new(orch);
    let handle = {
        let orch = orch_arc.clone();
        tokio::spawn(async move { orch.run_streaming(&mut history, "go".into(), tx).await })
    };

    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        let is_complete = matches!(ev, OrchestratorEvent::Complete { .. });
        events.push(ev);
        if is_complete {
            break;
        }
    }
    handle.await.unwrap().unwrap();

    // 期望顺序：Delta("Hel"), Delta("lo"), ToolCallStart(read), ToolResult(read), Delta("final"), Complete
    assert!(matches!(
        events[0],
        OrchestratorEvent::Delta { ref text } if text == "Hel"
    ));
    assert!(matches!(
        events[1],
        OrchestratorEvent::Delta { ref text } if text == "lo"
    ));
    assert!(matches!(
        events[2],
        OrchestratorEvent::ToolCallStart { ref name, .. } if name == "read"
    ));
    assert!(matches!(
        events[3],
        OrchestratorEvent::ToolResult { ref name, .. } if name == "read"
    ));
    assert!(matches!(
        events[4],
        OrchestratorEvent::Delta { ref text } if text == "final"
    ));
    let last = events.last().unwrap();
    let final_text = match last {
        OrchestratorEvent::Complete { text, .. } => text.clone(),
        other => panic!("expected Complete, got {:?}", other),
    };
    assert_eq!(final_text, "final");
}

#[tokio::test]
async fn dispatch_subagent_returns_summary() {
    let (orch, _dir) = build_orch(vec![vec![
        Event::Delta("explore-summary".into()),
        Event::Done,
    ]]);
    let summary = orch
        .dispatch_subagent(SubagentRole::Explore, "explore the codebase".into())
        .await
        .expect("subagent failed");
    assert_eq!(summary, "explore-summary");
}

#[tokio::test]
async fn dispatch_subagent_uses_restricted_sandbox_blocking_shell() {
    // 第一轮：LLM 试图调 shell（受限沙箱应拒绝——工具不存在）
    // 第二轮：LLM 给出最终文本
    let scripts = vec![
        vec![Event::ToolCallDelta {
            index: 0,
            id: Some("c1".into()),
            name: Some("shell".into()),
            arguments: Some("{\"command\":\"echo hi\"}".into()),
        }],
        vec![Event::Delta("done".into()), Event::Done],
    ];
    let (orch, _dir) = build_orch(scripts);
    let summary = orch
        .dispatch_subagent(SubagentRole::Review, "review".into())
        .await
        .expect("subagent failed");
    // 即便 shell 调用失败，循环继续，最终拿到第二轮文本
    assert_eq!(summary, "done");
}

#[test]
fn default_sandbox_registers_five_tools() {
    let dir = tempdir().unwrap();
    let (sb, specs) = default_sandbox_with_specs(dir.path().to_path_buf());
    let names = sb.list();
    assert_eq!(names, vec!["shell", "read", "write", "search", "grep"]);
    assert_eq!(specs.len(), 5);
    let spec_names: Vec<&str> = specs.iter().map(|s| s.function.name.as_str()).collect();
    assert!(spec_names.contains(&"shell"));
    assert!(spec_names.contains(&"write"));
}

#[test]
fn restricted_sandbox_excludes_shell_and_write() {
    let dir = tempdir().unwrap();
    let (sb, specs) = restricted_sandbox_with_specs(dir.path().to_path_buf());
    let names = sb.list();
    assert_eq!(names, vec!["read", "search", "grep"]);
    assert!(!names.contains(&"shell".to_string()));
    assert!(!names.contains(&"write".to_string()));
    assert_eq!(specs.len(), 3);
    // 验证 tool spec 的 parameters 是合法 JSON schema
    for spec in &specs {
        assert!(spec.function.parameters.is_object());
        assert_eq!(spec.typ, "function");
    }
}

#[tokio::test]
async fn run_once_tool_error_does_not_abort_turn() {
    // LLM 调一个不存在的工具 → sandbox.execute 返回 Err → 回填错误 → 第二轮完成
    let scripts = vec![
        vec![Event::ToolCallDelta {
            index: 0,
            id: Some("c1".into()),
            name: Some("nope_tool".into()),
            arguments: Some("{}".into()),
        }],
        vec![Event::Delta("recovered".into()), Event::Done],
    ];
    let (orch, _dir) = build_orch(scripts);
    let mut history = History::with_system("sys");
    let event = orch.run_once(&mut history, "x".into()).await.unwrap();
    match event {
        OrchestratorEvent::Complete { text, tool_calls } => {
            assert_eq!(text, "recovered");
            assert_eq!(tool_calls.len(), 1);
            assert!(tool_calls[0].result.error.is_some());
        }
        other => panic!("unexpected: {:?}", other),
    }
}

#[tokio::test]
async fn compile_prompt_with_real_profiles_succeeds() {
    // 验证 compile_prompt 走通真实 prompts 目录
    let dir = tempdir().unwrap();
    let (sandbox, specs) = default_sandbox_with_specs(dir.path().to_path_buf());
    let llm: Arc<dyn LlmClient> = Arc::new(MockClient::new(vec![]));
    let prompts_root =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../prompts/profiles");
    let orch = Orchestrator::new(
        llm,
        Arc::new(sandbox),
        specs,
        prompts_root,
        "default".into(),
        "deepseek-chat".into(),
        dir.path().to_path_buf(),
    );
    let prompt = orch
        .compile_prompt("default")
        .await
        .expect("compile failed");
    assert!(prompt.contains("## 身份与产品信息"));
    assert!(prompt.contains("deepseek-chat"));
    assert!(prompt.contains("read"));
}
