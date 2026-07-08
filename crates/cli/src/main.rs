//! ForgeClaw CLI 入口：clap 命令解析 + tracing 初始化 + 分发到 `commands`。

mod commands;
mod config;

use clap::{Parser, Subcommand};

use crate::config::Config;

/// ForgeClaw — AI 编码助手 CLI。
#[derive(Debug, Parser)]
#[command(name = "forgeclaw", version, about = "ForgeClaw AI 编码助手 CLI")]
struct Cli {
    /// 默认 profile 名（覆盖配置文件）。
    #[arg(short, long)]
    profile: Option<String>,

    /// 默认模型名（覆盖配置文件）。
    #[arg(short, long)]
    model: Option<String>,

    /// 启用调试日志。
    #[arg(short, long)]
    verbose: bool,

    #[command(subcommand)]
    cmd: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 进入交互式 REPL 对话（流式输出）。
    Chat(ChatArgs),
    /// 单次执行一个任务。
    Run(RunArgs),
    /// 提示词编译与章节查看。
    Prompt(PromptArgs),
    /// 工具列表与直接执行。
    Tool(ToolArgs),
    /// 启动 Web 服务（REST + WebSocket）。
    Web(WebArgs),
    /// 查看与修改本地配置（~/.forgeclaw/config.toml）。
    Config(ConfigArgs),
}

#[derive(Debug, Parser)]
struct ChatArgs {
    /// 可选：单次对话消息；不提供则进入 REPL。
    message: Option<String>,
}

#[derive(Debug, Parser)]
struct RunArgs {
    /// 任务描述。
    task: String,

    /// 自动应用工具调用（不询问确认）。
    #[arg(long)]
    auto_apply: bool,
}

#[derive(Debug, Parser)]
struct PromptArgs {
    #[command(subcommand)]
    action: PromptAction,
}

#[derive(Debug, Subcommand)]
enum PromptAction {
    /// 编译指定 profile 为完整 system prompt。
    Compile {
        /// profile 名（默认用全局 --profile 或配置文件）。
        #[arg(long)]
        profile: Option<String>,
    },
    /// 列出当前 profile 引用的章节。
    ListSections,
}

#[derive(Debug, Parser)]
struct ToolArgs {
    #[command(subcommand)]
    action: ToolAction,
}

#[derive(Debug, Subcommand)]
enum ToolAction {
    /// 列出已注册工具。
    List,
    /// 直接执行某个工具。
    Exec {
        /// 工具名（shell/read/write/search/grep）。
        tool: String,

        /// 工具参数（单个 JSON 对象透传，或按工具名映射字段）。
        args: Vec<String>,
    },
}

#[derive(Debug, Parser)]
struct WebArgs {
    /// 监听主机（默认仅回环，避免暴露到公网）。
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// 监听端口。
    #[arg(long, default_value_t = 8080)]
    port: u16,
}

#[derive(Debug, Parser)]
struct ConfigArgs {
    #[command(subcommand)]
    action: ConfigAction,
}

#[derive(Debug, Subcommand)]
enum ConfigAction {
    /// 显示当前配置（api_key 脱敏）。
    Show,

    /// 写入一项配置到磁盘。
    Set {
        /// 配置键（api_key/base_url/model/prompts_root/working_dir/profile）。
        key: String,

        /// 配置值。
        value: String,
    },

    /// 写入默认配置模板到 ~/.forgeclaw/config.toml。
    Init,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let mut cfg = Config::load()?;
    if let Some(p) = cli.profile {
        cfg.profile = p;
    }
    if let Some(m) = cli.model {
        cfg.model = m;
    }

    match cli.cmd {
        Command::Chat(a) => commands::run_chat(cfg, a.message).await,
        Command::Run(a) => commands::run_run(cfg, a.task, a.auto_apply).await,
        Command::Prompt(a) => match a.action {
            PromptAction::Compile { profile } => {
                let p = profile.unwrap_or_else(|| cfg.profile.clone());
                commands::run_prompt_compile(cfg, p).await
            }
            PromptAction::ListSections => commands::run_prompt_list(cfg).await,
        },
        Command::Tool(a) => match a.action {
            ToolAction::List => commands::run_tool_list(cfg).await,
            ToolAction::Exec { tool, args } => commands::run_tool_exec(cfg, tool, args).await,
        },
        Command::Web(a) => commands::run_web(cfg, a.host, a.port).await,
        Command::Config(a) => match a.action {
            ConfigAction::Show => commands::run_config_show(&cfg),
            ConfigAction::Set { key, value } => commands::run_config_set(&key, &value),
            ConfigAction::Init => commands::run_config_init(),
        },
    }
}

/// 初始化 tracing：优先读 RUST_LOG，否则按 verbose 选默认级别。
fn init_tracing(verbose: bool) {
    let default = if verbose {
        "forgeclaw=debug"
    } else {
        "forgeclaw=warn"
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}
