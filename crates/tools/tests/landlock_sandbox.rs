//! 沙箱 landlock 集成测试。
//!
//! - Linux：验证 landlock 阻止 `cd / && touch /tmp/...` 这类越界写操作。
//! - 非 Linux：验证现有 cwd 检查仍然生效。

use forgeclaw_tools::{auto_confirm, Sandbox, ShellTool};
use serde_json::json;
use tempfile::tempdir;

#[cfg(target_os = "linux")]
async fn landlock_is_available() -> bool {
    let probe = "/tmp/fc_landlock_probe_marker";
    let _ = std::fs::remove_file(probe);

    let dir = tempdir().unwrap();
    let mut sb = Sandbox::new(dir.path().to_path_buf(), auto_confirm());
    sb.register(Box::new(ShellTool::new(dir.path().to_path_buf())));

    let _ = sb
        .execute("shell", json!({"command": "cd / && touch /tmp/fc_landlock_probe_marker"}))
        .await;

    let available = !std::path::Path::new(probe).exists();
    let _ = std::fs::remove_file(probe);
    available
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn linux_landlock_blocks_write_outside_working_dir() {
    if !landlock_is_available().await {
        eprintln!("landlock not available in this environment, skipping test");
        return;
    }

    let dir = tempdir().unwrap();
    let mut sb = Sandbox::new(dir.path().to_path_buf(), auto_confirm());
    sb.register(Box::new(ShellTool::new(dir.path().to_path_buf())));

    let marker = "/tmp/fc_landlock_outside_test_marker";
    let _ = std::fs::remove_file(marker);

    let r = sb
        .execute(
            "shell",
            json!({"command": "cd / && touch /tmp/fc_landlock_outside_test_marker"}),
        )
        .await
        .unwrap();

    assert!(
        !std::path::Path::new(marker).exists(),
        "landlock should have blocked write to /tmp, but the marker file was created; result={:?}",
        r
    );
    assert!(
        r.error.is_some(),
        "expected command to fail due to landlock, but got: {:?}",
        r
    );

    let _ = std::fs::remove_file(marker);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn linux_landlock_allows_write_inside_working_dir() {
    let dir = tempdir().unwrap();
    let mut sb = Sandbox::new(dir.path().to_path_buf(), auto_confirm());
    sb.register(Box::new(ShellTool::new(dir.path().to_path_buf())));

    let r = sb
        .execute(
            "shell",
            json!({"command": "touch inside.txt && ls inside.txt"}),
        )
        .await
        .unwrap();

    assert!(
        r.error.is_none(),
        "expected write inside working dir to succeed, got error: {:?}",
        r.error
    );
    assert!(dir.path().join("inside.txt").exists());
}

#[cfg(not(target_os = "linux"))]
#[tokio::test]
async fn non_linux_blocks_cwd_outside_working_dir() {
    let dir = tempdir().unwrap();
    let mut sb = Sandbox::new(dir.path().to_path_buf(), auto_confirm());
    sb.register(Box::new(ShellTool::new(dir.path().to_path_buf())));

    let r = sb
        .execute("shell", json!({"command": "pwd", "cwd": "/tmp"}))
        .await
        .unwrap();

    assert!(r.error.unwrap().contains("blocked"));
}
