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
            // R2-SRV-008：ToolCallRecord.input 携带解析后的入参。
            assert_eq!(tool_calls[0].input, serde_json::json!({"path":"a.txt"}));
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
async fn run_streaming_same_name_tool_calls_match_by_call_id() {
    // 并发两个同名 read 工具调用，验证 ToolCallStart/ToolResult 的 call_id 一一对应。
    let scripts = vec![
        vec![
            Event::ToolCallDelta {
                index: 0,
                id: Some("call_a".into()),
                name: Some("read".into()),
                arguments: Some("{\"path\":\"a.txt\"}".into()),
            },
            Event::ToolCallDelta {
                index: 1,
                id: Some("call_b".into()),
                name: Some("read".into()),
                arguments: Some("{\"path\":\"b.txt\"}".into()),
            },
        ],
        vec![Event::Delta("done".into()), Event::Done],
    ];
    let (orch, dir) = build_orch(scripts);
    std::fs::write(dir.path().join("a.txt"), "A").unwrap();
    std::fs::write(dir.path().join("b.txt"), "B").unwrap();

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

    let starts: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            OrchestratorEvent::ToolCallStart { call_id, name, .. } => {
                Some((call_id.clone(), name.clone()))
            }
            _ => None,
        })
        .collect();
    let results: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            OrchestratorEvent::ToolResult {
                call_id, result, ..
            } => Some((call_id.clone(), result.output.clone())),
            _ => None,
        })
        .collect();

    assert_eq!(starts.len(), 2);
    assert_eq!(results.len(), 2);
    let start_ids: std::collections::HashSet<_> = starts.iter().map(|(id, _)| id).collect();
    let result_ids: std::collections::HashSet<_> = results.iter().map(|(id, _)| id).collect();
    assert_eq!(start_ids, result_ids);

    for (id, output) in &results {
        let expected = if id == "call_a" { "A" } else { "B" };
        assert_eq!(output, expected, "call_id {id} result mismatch");
    }
}

#[tokio::test]
async fn run_streaming_stops_when_receiver_dropped() {
    // D-006/B-003：receiver 被 drop 后，run_streaming 应优雅停止并返回 Ok，
    // 而不是 panic 或返回 Err。
    let scripts = vec![vec![
        Event::Delta("a".into()),
        Event::Delta("b".into()),
        Event::Delta("c".into()),
    ]];
    let (orch, _dir) = build_orch(scripts);
    let mut history = History::with_system("sys");
    let (tx, mut rx) = mpsc::channel::<OrchestratorEvent>(2);
    let orch_arc = Arc::new(orch);
    let handle = {
        let orch = orch_arc.clone();
        tokio::spawn(async move { orch.run_streaming(&mut history, "go".into(), tx).await })
    };

    let ev = rx.recv().await.expect("first delta");
    assert!(
        matches!(ev, OrchestratorEvent::Delta { ref text } if text == "a"),
        "expected first delta, got {:?}",
        ev
    );
    drop(rx);

    let result = handle.await.expect("task panicked");
    assert!(
        result.is_ok(),
        "receiver dropped should return Ok, got {:?}",
        result
    );
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
    // R2-SRV-001：工具失败时 error 合并进 tool 消息 content，LLM 能看到错误而非空内容。
    // history 布局：system(0) + user(1) + assistant(2,tool_calls) + tool(3) + assistant(4,"recovered")
    let tool_msg = &history.messages()[3];
    assert_eq!(tool_msg.role, Role::Tool);
    assert!(
        tool_msg.content.starts_with("error:"),
        "tool content should embed error, got: {}",
        tool_msg.content
    );
}

#[tokio::test]
async fn run_once_llm_error_does_not_modify_history() {
    // R2-SRV-002：LLM 第二轮返回 Error，history 不应残留半截 tool_calls 导致下次 400。
    // 第一轮：LLM 要求调用 read（assistant + tool 消息进入 temp）
    // 第二轮：LLM 流内 Error → run_turn 提前返回 Error，temp 不写回 history。
    let scripts = vec![
        vec![Event::ToolCallDelta {
            index: 0,
            id: Some("c1".into()),
            name: Some("read".into()),
            arguments: Some("{\"path\":\"a.txt\"}".into()),
        }],
        vec![Event::Error("boom".into())],
    ];
    let (orch, dir) = build_orch(scripts);
    std::fs::write(dir.path().join("a.txt"), "content").unwrap();

    let mut history = History::with_system("sys");
    let event = orch.run_once(&mut history, "x".into()).await.unwrap();
    assert!(
        matches!(event, OrchestratorEvent::Error { .. }),
        "expected Error event, got {:?}",
        event
    );
    // history 仅含 system（user/assistant/tool 均在 temp 中，错误路径不写回）。
    assert_eq!(history.len(), 1);
}

#[tokio::test]
async fn run_once_propagates_llm_stream_error() {
    // D-004：LLM 流内 Error 事件应被 run_once 包装为 OrchestratorEvent::Error 返回。
    let scripts = vec![vec![Event::Error("stream-boom".into())]];
    let (orch, _dir) = build_orch(scripts);
    let mut history = History::with_system("sys");
    let event = orch.run_once(&mut history, "x".into()).await.unwrap();
    assert!(
        matches!(event, OrchestratorEvent::Error { .. }),
        "expected Error event, got {:?}",
        event
    );
    match event {
        OrchestratorEvent::Error { message } => {
            assert_eq!(message, "stream-boom");
        }
        other => panic!("expected Error event, got {:?}", other),
    }
}

#[tokio::test]
async fn run_once_invalid_tool_arguments_returns_tool_result_error() {
    // D-012：工具参数 JSON 解析失败时，ToolResult.error 应包含 "invalid tool input"。
    let scripts = vec![
        vec![Event::ToolCallDelta {
            index: 0,
            id: Some("c1".into()),
            name: Some("read".into()),
            arguments: Some("not-json".into()),
        }],
        vec![Event::Delta("done".into()), Event::Done],
    ];
    let (orch, _dir) = build_orch(scripts);
    let mut history = History::with_system("sys");
    let event = orch.run_once(&mut history, "x".into()).await.unwrap();
    match event {
        OrchestratorEvent::Complete { text, tool_calls } => {
            assert_eq!(text, "done");
            assert_eq!(tool_calls.len(), 1);
            let err = tool_calls[0]
                .result
                .error
                .as_ref()
                .expect("tool result should carry error");
            assert!(
                err.contains("invalid tool input"),
                "expected error to contain 'invalid tool input', got: {}",
                err
            );
        }
        other => panic!("expected Complete event, got {:?}", other),
    }
}

#[tokio::test]
async fn run_once_tool_error_feeds_error_into_history() {
    // 工具执行失败后，回填给 LLM 的 tool_msg.content 应包含错误描述，而不是空字符串。
    let scripts = vec![
        vec![Event::ToolCallDelta {
            index: 0,
            id: Some("c1".into()),
            name: Some("nope_tool".into()),
            arguments: Some("{}".into()),
        }],
        vec![Event::Delta("ok".into()), Event::Done],
    ];
    let (orch, _dir) = build_orch(scripts);
    let mut history = History::with_system("sys");
    orch.run_once(&mut history, "x".into()).await.unwrap();

    // system + user + assistant(tool_calls) + tool + assistant("ok") = 5
    assert_eq!(history.len(), 5);
    let tool_msg = &history.messages()[3];
    assert_eq!(tool_msg.role, Role::Tool);
    assert!(
        tool_msg.content.contains("error:"),
        "tool_msg.content should contain error description, got {:?}",
        tool_msg.content
    );
    assert!(tool_msg.content.contains("nope_tool"));
}

#[tokio::test]
async fn run_once_max_turns_exceeded_returns_error() {
    // LLM 每轮都返回工具调用，25 轮后应因 max_turns 超限返回 Error。
    let scripts: Vec<Vec<Event>> = (0..25)
        .map(|_| {
            vec![Event::ToolCallDelta {
                index: 0,
                id: Some("c1".into()),
                name: Some("read".into()),
                arguments: Some("{\"path\":\"a.txt\"}".into()),
            }]
        })
        .collect();
    let (orch, dir) = build_orch(scripts);
    std::fs::write(dir.path().join("a.txt"), "x").unwrap();

    let mut history = History::with_system("sys");
    let event = orch.run_once(&mut history, "loop".into()).await.unwrap();
    match event {
        OrchestratorEvent::Error { message } => assert_eq!(message, "max turns exceeded"),
        other => panic!("expected max turns error, got {:?}", other),
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

#[tokio::test]
async fn default_sandbox_denies_file_write_without_confirmation() {
    // C-005：server 模式默认沙箱不得自动放行 Confirm 级 FileWriteTool。
    let dir = tempdir().unwrap();
    let (sandbox, _specs) = default_sandbox_with_specs(dir.path().to_path_buf());
    let target_file = dir.path().join("should_not_exist.txt");

    let result = sandbox
        .execute(
            "write",
            serde_json::json!({
                "path": "should_not_exist.txt",
                "content": "written by auto-confirm"
            }),
        )
        .await
        .expect("sandbox execute returned Err");

    assert_eq!(
        result.error.as_deref(),
        Some("blocked: user denied"),
        "FileWriteTool 应因未显式确认而被拒绝，got {:?}",
        result
    );
    assert!(!target_file.exists(), "被拒绝后目标文件不应被创建");
}
