//! `Sandbox`：工具注册 + 分层安全调度（Critical 拦截 / Confirm 确认 / Allow 放行）。
//!
//! 平台相关说明：
//! - Linux：在 `ShellTool` 启动子进程前，通过 landlock（Linux 5.13+）将文件系统访问
//!   严格限制在工作目录内，并拒绝外联 TCP 连接（内核 6.7+）。不支持 landlock 的内核会
//!   优雅降级到现有的 `current_dir` + `is_within` 检查。
//! - 非 Linux：当前仅通过 `current_dir` 与 `is_within` 限制路径，尚未引入内核级沙箱。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use forgeclaw_core::model::{SafetyLevel, ToolResult};
use serde_json::Value;

use crate::{FileReadTool, FileWriteTool, GrepTool, SearchTool, ShellTool, Tool};

/// 异步确认器：`(tool_name, input) -> 是否放行`。
///
/// 由 `#[async_trait]` 展开为对象安全签名，可在 `Arc<dyn AsyncConfirmer>` 中使用。
#[async_trait]
pub trait AsyncConfirmer: Send + Sync {
    async fn confirm(&self, name: &str, input: &Value) -> bool;
}

/// 总是放行的确认器（供测试与默认场景）。
pub struct AutoConfirm;

#[async_trait]
impl AsyncConfirmer for AutoConfirm {
    async fn confirm(&self, _name: &str, _input: &Value) -> bool {
        true
    }
}

/// 返回一个总是放行的 [`AsyncConfirmer`]。
pub fn auto_confirm() -> Arc<dyn AsyncConfirmer> {
    Arc::new(AutoConfirm)
}

/// 将同步闭包包装为 [`AsyncConfirmer`]（不阻塞 runtime，闭包本身需立即返回）。
pub struct FnConfirmer<F> {
    f: F,
}

#[async_trait]
impl<F> AsyncConfirmer for FnConfirmer<F>
where
    F: Fn(&str, &Value) -> bool + Send + Sync + 'static,
{
    async fn confirm(&self, name: &str, input: &Value) -> bool {
        (self.f)(name, input)
    }
}

/// 用同步闭包构造确认器。
pub fn confirmer_from_fn<F>(f: F) -> Arc<dyn AsyncConfirmer>
where
    F: Fn(&str, &Value) -> bool + Send + Sync + 'static,
{
    Arc::new(FnConfirmer { f })
}

/// 基于 tokio channel 的测试确认器：外部通过 sender 注入 `true/false`。
pub struct ChannelConfirmer {
    responses: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<bool>>,
}

#[async_trait]
impl AsyncConfirmer for ChannelConfirmer {
    async fn confirm(&self, _name: &str, _input: &Value) -> bool {
        self.responses.lock().await.recv().await.unwrap_or(false)
    }
}

/// 构造 channel 确认器及其控制端。
pub fn channel_confirmer(
    buffer: usize,
) -> (Arc<dyn AsyncConfirmer>, tokio::sync::mpsc::Sender<bool>) {
    let (tx, rx) = tokio::sync::mpsc::channel(buffer);
    (
        Arc::new(ChannelConfirmer {
            responses: tokio::sync::Mutex::new(rx),
        }),
        tx,
    )
}

/// 工具沙箱：持有工作目录、工具集合与确认回调。
pub struct Sandbox {
    working_dir: PathBuf,
    tools: Vec<Box<dyn Tool>>,
    confirmer: Arc<dyn AsyncConfirmer>,
}

impl Sandbox {
    pub fn new(working_dir: PathBuf, confirmer: Arc<dyn AsyncConfirmer>) -> Self {
        Self {
            working_dir,
            tools: Vec::new(),
            confirmer,
        }
    }

    /// 注册一个工具。
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    /// 替换当前确认器，返回 `&mut self` 以便链式调用。
    pub fn with_confirmer(&mut self, confirmer: Arc<dyn AsyncConfirmer>) -> &mut Self {
        self.confirmer = confirmer;
        self
    }

    /// 已注册工具名列表。
    pub fn list(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.name().to_string()).collect()
    }

    /// 工作目录。
    pub fn working_dir(&self) -> &Path {
        &self.working_dir
    }

    /// 按 `tool_name` 路由执行：先 `check` 判定级别，再决定拦截/确认/放行。
    pub async fn execute(&self, tool_name: &str, input: Value) -> anyhow::Result<ToolResult> {
        let tool = self
            .tools
            .iter()
            .find(|t| t.name() == tool_name)
            .ok_or_else(|| anyhow::anyhow!("tool not found: {}", tool_name))?;
        match tool.check(&input).await {
            SafetyLevel::Critical => Ok(ToolResult {
                output: String::new(),
                error: Some("blocked: critical safety level".to_string()),
                duration_ms: 0,
            }),
            SafetyLevel::Confirm => {
                if self.confirmer.confirm(tool_name, &input).await {
                    tool.execute(input).await
                } else {
                    Ok(ToolResult {
                        output: String::new(),
                        error: Some("blocked: user denied".to_string()),
                        duration_ms: 0,
                    })
                }
            }
            SafetyLevel::Allow => tool.execute(input).await,
        }
    }

    /// 预置 5 工具（shell/read/write/search/grep）+ `auto_confirm` 的默认沙箱。
    pub fn default_for(working_dir: PathBuf) -> Self {
        let mut sb = Sandbox::new(working_dir.clone(), auto_confirm());
        sb.register(Box::new(ShellTool::new(working_dir.clone())));
        sb.register(Box::new(FileReadTool::new(working_dir.clone())));
        sb.register(Box::new(FileWriteTool::new(working_dir.clone())));
        sb.register(Box::new(SearchTool::new(working_dir.clone())));
        sb.register(Box::new(GrepTool::new(working_dir)));
        sb
    }
}

/// Linux 下使用 landlock 在**当前线程/进程**建立文件系统 + 网络沙箱。
///
/// 应在 `fork` 之后、`exec` 之前的子进程中调用（例如 `std::os::unix::process::CommandExt::pre_exec`），
/// 以免限制父进程。非 Linux 平台无此函数。
///
/// 策略：
/// - 工作目录：读、写、执行；
/// - `/` 及必要系统目录（`/bin`、`/usr`、`/lib` 等）：读 + 执行，仅用于加载 `sh`、动态库；
/// - `/dev/null`：读写，用于常见重定向；
/// - 不添加任何网络规则，因此所有 TCP 连接/绑定被拒绝（内核 6.7+）。
///
/// 若内核不支持 landlock，则记录警告并返回 `Ok(())`，由调用方继续执行现有 cwd 检查。
#[cfg(target_os = "linux")]
pub fn apply_landlock(working_dir: &Path) -> anyhow::Result<()> {
    use landlock::{
        Access, AccessFs, AccessNet, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
        RulesetStatus, ABI,
    };

    // ABI::V4 引入 TCP connect/bind；若内核不支持，BestEffort 兼容性会自动降级。
    let abi = ABI::V4;
    let fs_all = AccessFs::from_all(abi);
    let fs_rx = AccessFs::from_read(abi) | AccessFs::Execute;
    let net_all = AccessNet::from_all(abi);

    let mut ruleset = Ruleset::default()
        .handle_access(fs_all)?
        .handle_access(net_all)?
        .create()?;

    // 工作目录：完全访问。
    let wd_fd = PathFd::new(working_dir)
        .map_err(|e| anyhow::anyhow!("failed to open working_dir for landlock: {}", e))?;
    ruleset = ruleset.add_rule(PathBeneath::new(wd_fd, fs_all))?;

    // `/`：仅允许遍历，避免过度开放根目录列表。
    if let Ok(fd) = PathFd::new(Path::new("/")) {
        ruleset = ruleset.add_rule(PathBeneath::new(fd, AccessFs::Execute))?;
    }

    // 运行 /bin/sh、动态链接器及读取 /etc/ld.so.cache 等所需的系统路径。
    let system_paths: &[&Path] = &[
        Path::new("/bin"),
        Path::new("/sbin"),
        Path::new("/usr"),
        Path::new("/lib"),
        Path::new("/lib64"),
        Path::new("/usr/lib"),
        Path::new("/usr/lib64"),
        Path::new("/etc"),
    ];
    for p in system_paths {
        if let Ok(fd) = PathFd::new(p) {
            ruleset = ruleset.add_rule(PathBeneath::new(fd, fs_rx))?;
        }
    }

    // /dev/null：允许读写，供常见重定向使用；/dev 仅允许遍历。
    if let Ok(fd) = PathFd::new(Path::new("/dev")) {
        ruleset = ruleset.add_rule(PathBeneath::new(fd, AccessFs::Execute))?;
    }
    if let Ok(fd) = PathFd::new(Path::new("/dev/null")) {
        ruleset = ruleset.add_rule(PathBeneath::new(
            fd,
            AccessFs::from_read(abi) | AccessFs::WriteFile,
        ))?;
    }

    let status = ruleset.restrict_self()?;
    match status.ruleset {
        RulesetStatus::FullyEnforced => {
            tracing::debug!(path = %working_dir.display(), "landlock fully enforced");
        }
        RulesetStatus::PartiallyEnforced => {
            tracing::warn!(path = %working_dir.display(), "landlock partially enforced");
        }
        RulesetStatus::NotEnforced => {
            tracing::warn!(
                "landlock not supported by this kernel (requires Linux 5.13+); falling back to cwd check"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[tokio::test]
    async fn default_for_registers_five_tools() {
        let dir = tempdir().unwrap();
        let sb = Sandbox::default_for(dir.path().to_path_buf());
        assert_eq!(sb.list(), vec!["shell", "read", "write", "search", "grep"]);
    }

    #[tokio::test]
    async fn routes_to_correct_tool() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), "content").unwrap();
        let mut sb = Sandbox::new(dir.path().to_path_buf(), auto_confirm());
        sb.register(Box::new(FileReadTool::new(dir.path().to_path_buf())));
        sb.register(Box::new(ShellTool::new(dir.path().to_path_buf())));
        let r = sb.execute("read", json!({"path": "x.txt"})).await.unwrap();
        assert_eq!(r.output, "content");
        let r2 = sb
            .execute("shell", json!({"command": "echo hi"}))
            .await
            .unwrap();
        assert!(r2.output.contains("hi"));
    }

    #[tokio::test]
    async fn critical_blocks_without_confirmer() {
        let dir = tempdir().unwrap();
        let mut sb = Sandbox::new(dir.path().to_path_buf(), auto_confirm());
        sb.register(Box::new(ShellTool::new(dir.path().to_path_buf())));
        let r = sb
            .execute("shell", json!({"command": "rm -rf /"}))
            .await
            .unwrap();
        assert!(r.error.unwrap().contains("blocked"));
    }

    #[tokio::test]
    async fn confirm_false_blocks() {
        let dir = tempdir().unwrap();
        let confirmer = confirmer_from_fn(|_, _| false);
        let mut sb = Sandbox::new(dir.path().to_path_buf(), confirmer);
        sb.register(Box::new(FileWriteTool::new(dir.path().to_path_buf())));
        let r = sb
            .execute("write", json!({"path": "y.txt", "content": "z"}))
            .await
            .unwrap();
        assert!(r.error.unwrap().contains("blocked"));
        assert!(!dir.path().join("y.txt").exists());
    }

    #[tokio::test]
    async fn confirm_true_proceeds() {
        let dir = tempdir().unwrap();
        let mut sb = Sandbox::new(dir.path().to_path_buf(), auto_confirm());
        sb.register(Box::new(FileWriteTool::new(dir.path().to_path_buf())));
        let r = sb
            .execute("write", json!({"path": "y.txt", "content": "z"}))
            .await
            .unwrap();
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("y.txt")).unwrap(),
            "z"
        );
    }

    #[tokio::test]
    async fn unknown_tool_errors() {
        let dir = tempdir().unwrap();
        let sb = Sandbox::default_for(dir.path().to_path_buf());
        let res = sb.execute("nope", json!({})).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn allow_tool_skips_confirmer_even_if_false() {
        // ShellTool 对 `ls` 为 Allow，即便 confirmer 总返回 false 也应执行。
        let dir = tempdir().unwrap();
        let confirmer = confirmer_from_fn(|_, _| false);
        let mut sb = Sandbox::new(dir.path().to_path_buf(), confirmer);
        sb.register(Box::new(ShellTool::new(dir.path().to_path_buf())));
        let r = sb
            .execute("shell", json!({"command": "echo ok"}))
            .await
            .unwrap();
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(r.output.contains("ok"));
    }

    #[tokio::test]
    async fn channel_confirmer_true_allows() {
        let dir = tempdir().unwrap();
        let (confirmer, tx) = channel_confirmer(2);
        let mut sb = Sandbox::new(dir.path().to_path_buf(), confirmer);
        sb.register(Box::new(FileWriteTool::new(dir.path().to_path_buf())));
        let path = dir.path().join("y.txt");
        let task = sb.execute("write", json!({"path": "y.txt", "content": "z"}));
        tx.send(true).await.unwrap();
        let r = task.await.unwrap();
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "z");
    }

    #[tokio::test]
    async fn channel_confirmer_false_blocks() {
        let dir = tempdir().unwrap();
        let (confirmer, tx) = channel_confirmer(2);
        let mut sb = Sandbox::new(dir.path().to_path_buf(), confirmer);
        sb.register(Box::new(FileWriteTool::new(dir.path().to_path_buf())));
        let task = sb.execute("write", json!({"path": "y.txt", "content": "z"}));
        tx.send(false).await.unwrap();
        let r = task.await.unwrap();
        assert!(r.error.unwrap().contains("blocked"));
        assert!(!dir.path().join("y.txt").exists());
    }
}
