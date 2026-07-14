//! CLI 子命令实现：chat REPL（流式着色）/ run / prompt / tool / web / config。
//!
//! 设计要点：
//! - `chat` 与 `web` 用 `forgeclaw_server::build_orchestrator`（auto_confirm）；
//!   `run` 默认 confirm 模式，在 cli 内自建 orchestrator（带 stdin 确认器）。
//! - 流式着色直接用 ANSI 转义，不引入 colored。
//! - `/model` 切换需重建 orchestrator（`Orchestrator.model` 无 setter）。

use std::collections::HashMap;
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::bail;
use async_trait::async_trait;
use forgeclaw_core::prompt::PromptEngine;
use forgeclaw_llm::{History, LlmClient, OpenAiClient};
use forgeclaw_server::{
    build_orchestrator as build_server_orchestrator, default_sandbox_with_specs, run as server_run,
    AppState, Orchestrator, OrchestratorConfig, OrchestratorEvent, UserStore,
};
use forgeclaw_tools::{AsyncConfirmer, Sandbox};

use crate::config::Config;

// ============ ANSI 颜色（直接转义，不引入 colored） ============

const CYAN: &str = "\x1b[36m";
const YELLOW: &str = "\x1b[33m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const RESET: &str = "\x1b[0m";

// ============ Orchestrator 构建 ============

/// 构建 orchestrator。`auto_apply=true` 用 server 工厂（auto_confirm）；
/// `false` 在 cli 内自建带 stdin 确认器的沙箱。
fn build_orchestrator(cfg: &Config, auto_apply: bool) -> anyhow::Result<Arc<Orchestrator>> {
    cfg.validate()?;
    if auto_apply {
        let orch = build_server_orchestrator(OrchestratorConfig {
            base_url: cfg.base_url.clone(),
            api_key: cfg.api_key.clone(),
            prompts_root: cfg.prompts_root.clone(),
            working_dir: cfg.working_dir.clone(),
            model: cfg.model.clone(),
            profile: cfg.profile.clone(),
        })?;
        return Ok(orch);
    }
    build_orchestrator_confirm(cfg)
}

/// CLI confirm 模式使用的 stdin 确认器：在 `spawn_blocking` 中读取用户输入，
/// 避免阻塞 async runtime。
struct StdinConfirmer;

#[async_trait]
impl AsyncConfirmer for StdinConfirmer {
    async fn confirm(&self, name: &str, input: &serde_json::Value) -> bool {
        let name = name.to_string();
        let input = input.clone();
        tokio::task::spawn_blocking(move || {
            eprintln!();
            eprintln!("{YELLOW}[确认] 工具调用: {name}{RESET}");
            eprintln!(
                "  输入: {}",
                serde_json::to_string(&input).unwrap_or_default()
            );
            eprint!("允许执行? [y/N] ");
            let _ = io::stderr().flush();
            let mut line = String::new();
            match io::stdin().read_line(&mut line) {
                Ok(_) => line.trim().eq_ignore_ascii_case("y"),
                Err(_) => false,
            }
        })
        .await
        .unwrap_or(false)
    }
}

/// confirm 模式：复用 server 默认沙箱与工具 spec 描述，仅替换确认器为 stdin。
fn build_orchestrator_confirm(cfg: &Config) -> anyhow::Result<Arc<Orchestrator>> {
    let llm: Arc<dyn LlmClient> = Arc::new(OpenAiClient::new(
        cfg.base_url.clone(),
        cfg.api_key.clone(),
    )?);
    let engine = PromptEngine::new(cfg.prompts_root.clone());
    let model = engine.resolve_model(&cfg.profile, &cfg.model)?;
    let working_dir = cfg.working_dir.clone();
    let (mut sandbox, specs) = default_sandbox_with_specs(working_dir.clone());
    sandbox.with_confirmer(Arc::new(StdinConfirmer));
    Ok(Arc::new(Orchestrator::new(
        llm,
        Arc::new(sandbox),
        specs,
        cfg.prompts_root.clone(),
        cfg.profile.clone(),
        model,
        working_dir,
    )))
}

/// 与 orchestrator.prompt_vars 一致的变量注入（tools/model/cwd）。
fn prompt_vars(cfg: &Config) -> HashMap<&'static str, String> {
    let tools = Sandbox::default_for(cfg.working_dir.clone())
        .list()
        .join(", ");
    let mut v = HashMap::new();
    v.insert("tools", tools);
    v.insert("model", cfg.model.clone());
    v.insert("cwd", cfg.working_dir.display().to_string());
    v
}

// ============ chat ============

pub async fn run_chat(cfg: Config, message: Option<String>) -> anyhow::Result<()> {
    let mut orch = build_orchestrator(&cfg, true)?;
    let mut history = History::new();

    if let Some(msg) = message {
        let event = orch.run_once(&mut history, msg).await?;
        print_final_event(&event);
        return Ok(());
    }

    let mut rl = rustyline::DefaultEditor::new()?;
    println!("ForgeClaw REPL — /help 查看命令，/exit 退出");
    let mut current_cfg = cfg;
    loop {
        let line = match rl.readline("fc> ") {
            Ok(l) => l,
            Err(rustyline::error::ReadlineError::Interrupted) => continue,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => return Err(e.into()),
        };
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        let _ = rl.add_history_entry(&trimmed);

        if trimmed == "/exit" || trimmed == "/quit" {
            break;
        }
        if trimmed == "/help" {
            print_repl_help();
            continue;
        }
        if trimmed == "/clear" {
            history = History::new();
            println!("(会话已清空)");
            continue;
        }
        if trimmed == "/prompt" {
            match orch.compile_prompt(&current_cfg.profile).await {
                Ok(p) => println!("{p}"),
                Err(e) => eprintln!("{RED}编译失败: {e}{RESET}"),
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("/tool ") {
            let cmd = rest.trim();
            if cmd == "ls" || cmd == "list" {
                for t in Sandbox::default_for(current_cfg.working_dir.clone()).list() {
                    println!("{t}");
                }
            } else {
                println!("未知 /tool 子命令: {cmd}（可用: ls）");
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("/model ") {
            let new_model = rest.trim().to_string();
            if new_model.is_empty() {
                println!("用法: /model <name>");
                continue;
            }
            let mut next_cfg = current_cfg.clone();
            next_cfg.model = new_model.clone();
            match build_orchestrator(&next_cfg, true) {
                Ok(new_orch) => {
                    orch = new_orch;
                    current_cfg = next_cfg;
                    println!("(模型已切换为 {new_model})");
                }
                Err(e) => println!("切换失败: {e}"),
            }
            continue;
        }
        if trimmed.starts_with('/') {
            println!("未知命令: {trimmed}（/help 查看可用命令）");
            continue;
        }

        if let Err(e) = run_streaming_turn(orch.clone(), &mut history, trimmed).await {
            eprintln!("{RED}出错: {e}{RESET}");
        }
    }
    Ok(())
}

fn print_repl_help() {
    println!("REPL 命令:");
    println!("  /help              显示本帮助");
    println!("  /exit | /quit      退出");
    println!("  /clear             清空当前会话");
    println!("  /prompt            显示当前编译的 system prompt");
    println!("  /tool ls           列出可用工具");
    println!("  /model <name>      切换 LLM 模型");
    println!("直接输入文本即与助手对话（流式输出）。");
}

/// 驱动一轮流式对话：用 `tokio::join!` 在同一任务上并发跑 `run_streaming`（生产事件）
/// 与 receiver 排空（着色打印）。history 借用传入，调用方始终持有，出错也不丢会话。
async fn run_streaming_turn(
    orch: Arc<Orchestrator>,
    history: &mut History,
    msg: String,
) -> anyhow::Result<()> {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<OrchestratorEvent>(128);
    let producer = async move { orch.run_streaming(history, msg, tx).await };
    let consumer = async {
        while let Some(ev) = rx.recv().await {
            print_stream_event(&ev);
            if matches!(
                ev,
                OrchestratorEvent::Complete { .. } | OrchestratorEvent::Error { .. }
            ) {
                break;
            }
        }
    };
    let (res, ()) = tokio::join!(producer, consumer);
    res
}

fn print_stream_event(ev: &OrchestratorEvent) {
    match ev {
        OrchestratorEvent::Delta { text } => {
            print!("{CYAN}{text}{RESET}");
            let _ = io::stdout().flush();
        }
        OrchestratorEvent::ToolCallStart { name, input, .. } => {
            println!("\n{YELLOW}[tool] {name}: {input}{RESET}");
            let _ = io::stdout().flush();
        }
        OrchestratorEvent::ToolResult { name, result, .. } => {
            if let Some(err) = &result.error {
                println!("{RED}[result] {name}: error: {err}{RESET}");
            } else {
                println!(
                    "{GREEN}[result] {name}: {}{RESET}",
                    truncate(&result.output, 200)
                );
            }
            let _ = io::stdout().flush();
        }
        OrchestratorEvent::Complete { .. } => {
            println!();
            let _ = io::stdout().flush();
        }
        OrchestratorEvent::Error { message } => {
            eprintln!("{RED}[error] {message}{RESET}");
        }
    }
}

fn print_final_event(ev: &OrchestratorEvent) {
    match ev {
        OrchestratorEvent::Complete { text, .. } => println!("{text}"),
        OrchestratorEvent::Error { message } => eprintln!("{RED}[error] {message}{RESET}"),
        other => eprintln!("{RED}[unexpected event: {other:?}]{RESET}"),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("...");
    out
}

// ============ run ============

pub async fn run_run(cfg: Config, task: String, auto_apply: bool) -> anyhow::Result<()> {
    let orch = build_orchestrator(&cfg, auto_apply)?;
    let mut history = History::new();
    let event = orch.run_once(&mut history, task).await?;
    print_final_event(&event);
    Ok(())
}

// ============ prompt ============

pub async fn run_prompt_compile(cfg: Config, profile: String) -> anyhow::Result<()> {
    let engine = PromptEngine::new(cfg.prompts_root.clone());
    let vars = prompt_vars(&cfg);
    let prompt = engine.compile(&profile, &vars).await?;
    print!("{prompt}");
    if !prompt.ends_with('\n') {
        println!();
    }
    Ok(())
}

pub async fn run_prompt_list(cfg: Config) -> anyhow::Result<()> {
    let engine = PromptEngine::new(cfg.prompts_root.clone());
    let sections = engine.list_sections(&cfg.profile)?;
    if sections.is_empty() {
        println!("(无启用的 section)");
        return Ok(());
    }
    for s in sections {
        println!("{}. {} ({})", s.order, s.title, s.id);
    }
    Ok(())
}

// ============ tool ============

pub async fn run_tool_list(cfg: Config) -> anyhow::Result<()> {
    let sb = Sandbox::default_for(cfg.working_dir.clone());
    for t in sb.list() {
        println!("{t}");
    }
    Ok(())
}

pub async fn run_tool_exec(cfg: Config, tool: String, args: Vec<String>) -> anyhow::Result<()> {
    let sb = Sandbox::default_for(cfg.working_dir.clone());
    let input = build_tool_input(&tool, &args);
    let result = sb.execute(&tool, input).await?;
    let mut failed = false;
    if let Some(err) = &result.error {
        eprintln!("{RED}error: {err}{RESET}");
        failed = true;
    }
    if !result.output.is_empty() {
        print!("{}", result.output);
        if !result.output.ends_with('\n') {
            println!();
        }
        let _ = io::stdout().flush();
    }
    if failed {
        std::process::exit(1);
    }
    Ok(())
}

/// 把命令行 args 构造为工具 input。
/// 单个 JSON 对象参数直接用作 input；否则按工具名映射字段。
fn build_tool_input(tool: &str, args: &[String]) -> serde_json::Value {
    use serde_json::json;
    if args.len() == 1 {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&args[0]) {
            if v.is_object() {
                return v;
            }
        }
    }
    match tool {
        "shell" => json!({ "command": args.join(" ") }),
        "read" => json!({ "path": args.first().cloned().unwrap_or_default() }),
        "write" => json!({
            "path": args.first().cloned().unwrap_or_default(),
            "content": args.iter().skip(1).cloned().collect::<Vec<_>>().join(" ")
        }),
        "search" => json!({ "pattern": args.first().cloned().unwrap_or_default() }),
        "grep" => {
            let mut v = json!({ "pattern": args.first().cloned().unwrap_or_default() });
            if let Some(p) = args.get(1) {
                v["path"] = serde_json::Value::String(p.clone());
            }
            v
        }
        _ => json!({ "args": args }),
    }
}

// ============ web ============

pub async fn run_web(cfg: Config, host: String, port: u16) -> anyhow::Result<()> {
    let orch = build_orchestrator(&cfg, true)?;
    let users = resolve_users(&cfg);
    if users.is_empty() {
        bail!(
            "未配置 web 用户：请运行 `forgeclaw config init` 生成默认配置与随机 token，\
             或设置环境变量 FORGECLAW_USERS=name:token[,name:token]"
        );
    }
    let is_loopback = matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1");
    if !is_loopback {
        for (_name, token) in &users {
            if token.is_empty() || token == "change-me" || token == "local-token" {
                bail!(
                    "绑定到非回环地址 {host} 时检测到弱 token（change-me/local-token/空），\
                     拒绝启动。请运行 `forgeclaw config init` 生成随机 token 或手动修改配置。"
                );
            }
        }
    }
    let user_store = UserStore::from_config(users.clone());
    let allowed_origins = resolve_allowed_origins(&cfg, &host, port);
    let state = AppState::new(orch, user_store, allowed_origins);
    let addr = SocketAddr::new(host.parse()?, port);
    println!("ForgeClaw Web 服务已启动: http://{host}:{port}");
    println!(
        "可用用户: {}",
        users
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "登录 POST /api/auth/login {{name, token}}；受保护路由需 Authorization: Bearer <token>"
    );
    server_run(addr, state).await
}

/// 解析 web 用户：env `FORGECLAW_USERS` > 配置文件 users；无则返回空 vec。
fn resolve_users(cfg: &Config) -> Vec<(String, String)> {
    let env_raw = std::env::var("FORGECLAW_USERS").unwrap_or_default();
    if !env_raw.trim().is_empty() {
        env_raw
            .split(',')
            .filter_map(|e| {
                let (n, t) = e.trim().split_once(':')?;
                let n = n.trim().to_string();
                let t = t.trim().to_string();
                if n.is_empty() || t.is_empty() {
                    return None;
                }
                Some((n, t))
            })
            .collect()
    } else {
        cfg.users.clone()
    }
}

/// 解析 CORS 白名单：`config.allowed_origins` 非空则用之，否则用代码默认值
/// （vite dev 5173）；非回环 host 追加 `http://{host}:{port}`（SRV-001）。
fn resolve_allowed_origins(cfg: &Config, host: &str, port: u16) -> Vec<String> {
    let mut origins = if cfg.allowed_origins.is_empty() {
        vec![
            "http://127.0.0.1:5173".to_string(),
            "http://localhost:5173".to_string(),
        ]
    } else {
        cfg.allowed_origins.clone()
    };
    let is_loopback = matches!(host, "127.0.0.1" | "localhost" | "::1");
    if !is_loopback {
        let origin = format!("http://{host}:{port}");
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }
    origins
}

// ============ config ============

pub fn run_config_show(cfg: &Config) -> anyhow::Result<()> {
    println!("api_key     : {}", cfg.masked_api_key());
    println!("base_url    : {}", cfg.base_url);
    println!("model       : {}", cfg.model);
    println!("prompts_root: {}", cfg.prompts_root.display());
    println!("working_dir : {}", cfg.working_dir.display());
    println!("profile     : {}", cfg.profile);
    let users = cfg
        .users
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "users       : {}",
        if users.is_empty() { "(无)" } else { &users }
    );
    Ok(())
}

pub fn run_config_init() -> anyhow::Result<()> {
    let cfg = Config::default_for_init();
    cfg.save()?;
    match crate::config::config_path() {
        Some(p) => println!("已写入默认配置: {}", p.display()),
        None => println!("已写入默认配置"),
    }
    if let Some((_, token)) = cfg.users.first() {
        println!("已生成随机登录 token（请妥善保存，仅显示一次）: {token}");
    }
    println!("请编辑该文件填入 api_key，或设置环境变量 DEEPSEEK_API_KEY。");
    Ok(())
}

pub fn run_config_set(key: &str, value: &str) -> anyhow::Result<()> {
    let mut cfg = crate::config::load_file_or_defaults();
    match key {
        "api_key" => cfg.api_key = value.to_string(),
        "base_url" => cfg.base_url = value.to_string(),
        "model" => cfg.model = value.to_string(),
        "prompts_root" => cfg.prompts_root = PathBuf::from(value),
        "working_dir" => cfg.working_dir = PathBuf::from(value),
        "profile" => cfg.profile = value.to_string(),
        _ => bail!(
            "未知配置项: {key}（支持: api_key, base_url, model, prompts_root, working_dir, profile）"
        ),
    }
    cfg.save()?;
    if key == "api_key" {
        println!("已设置 {key} = {}", cfg.masked_api_key());
    } else {
        println!("已设置 {key} = {value}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_tool_input_json_object_passthrough() {
        let v = build_tool_input("shell", &["{\"command\":\"echo hi\"}".to_string()]);
        assert_eq!(v["command"], "echo hi");
    }

    #[test]
    fn build_tool_input_shell_joins_args() {
        let v = build_tool_input("shell", &["echo".to_string(), "hello world".to_string()]);
        assert_eq!(v["command"], "echo hello world");
    }

    #[test]
    fn build_tool_input_read_path() {
        let v = build_tool_input("read", &["a.txt".to_string()]);
        assert_eq!(v["path"], "a.txt");
    }

    #[test]
    fn build_tool_input_write_path_content() {
        let v = build_tool_input(
            "write",
            &[
                "out.txt".to_string(),
                "hello".to_string(),
                "there".to_string(),
            ],
        );
        assert_eq!(v["path"], "out.txt");
        assert_eq!(v["content"], "hello there");
    }

    #[test]
    fn build_tool_input_grep_optional_path() {
        let v = build_tool_input("grep", &["foo".to_string(), "src".to_string()]);
        assert_eq!(v["pattern"], "foo");
        assert_eq!(v["path"], "src");
    }

    #[test]
    fn build_tool_input_unknown_falls_back_to_args() {
        let v = build_tool_input("nope", &["x".to_string(), "y".to_string()]);
        assert_eq!(v["args"][0], "x");
        assert_eq!(v["args"][1], "y");
    }

    #[test]
    fn truncate_short_unchanged() {
        assert_eq!(truncate("abc", 10), "abc");
    }

    #[test]
    fn truncate_long_appends_ellipsis() {
        let s = "a".repeat(50);
        let t = truncate(&s, 5);
        assert_eq!(t.chars().count(), 8);
        assert!(t.ends_with("..."));
    }

    #[test]
    fn resolve_users_empty_when_no_config() {
        // 无 env 无配置 → 空 vec（不再兜底 local-token）
        std::env::remove_var("FORGECLAW_USERS");
        let cfg = Config::default();
        let u = resolve_users(&cfg);
        assert!(u.is_empty());
    }

    #[test]
    fn resolve_users_uses_config_users_when_no_env() {
        std::env::remove_var("FORGECLAW_USERS");
        let cfg = Config {
            users: vec![("alice".into(), "t1".into())],
            ..Config::default()
        };
        let u = resolve_users(&cfg);
        assert_eq!(u[0].0, "alice");
    }

    #[test]
    fn resolve_allowed_origins_default_loopback() {
        let cfg = Config::default();
        let origins = resolve_allowed_origins(&cfg, "127.0.0.1", 8080);
        assert!(origins.contains(&"http://127.0.0.1:5173".to_string()));
        assert!(origins.contains(&"http://localhost:5173".to_string()));
        // 回环 host 不追加自身端口
        assert!(!origins.contains(&"http://127.0.0.1:8080".to_string()));
    }

    #[test]
    fn resolve_allowed_origins_non_loopback_appends_host_port() {
        let cfg = Config::default();
        let origins = resolve_allowed_origins(&cfg, "0.0.0.0", 8080);
        assert!(origins.contains(&"http://0.0.0.0:8080".to_string()));
    }

    #[test]
    fn resolve_allowed_origins_uses_config_when_non_empty() {
        let cfg = Config {
            allowed_origins: vec!["http://custom.example".into()],
            ..Config::default()
        };
        let origins = resolve_allowed_origins(&cfg, "127.0.0.1", 8080);
        assert_eq!(origins, vec!["http://custom.example".to_string()]);
    }

    #[tokio::test]
    async fn tool_list_returns_five_tools() {
        let dir = std::env::temp_dir();
        let sb = Sandbox::default_for(dir);
        let names = sb.list();
        assert_eq!(names.len(), 5);
        assert!(names.contains(&"shell".to_string()));
    }
}
