//! CLI 配置：从环境变量与 `~/.forgeclaw/config.toml` 加载，回退默认值。
//!
//! 优先级：环境变量 `DEEPSEEK_API_KEY` / `FORGECLAW_API_KEY` > 配置文件 > 默认值。
//! `prompts_root` 默认指向 `prompts/profiles`（`PromptEngine` 期望含 `{profile}.toml`
//! 的目录，section 路径相对其父目录解析）。

use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// CLI 运行配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// LLM API Key（DeepSeek/OpenAI 兼容）。空表示未配置。
    #[serde(default)]
    pub api_key: String,

    /// OpenAI 兼容端点。
    #[serde(default = "default_base_url")]
    pub base_url: String,

    /// 默认模型名。
    #[serde(default = "default_model")]
    pub model: String,

    /// profiles 根目录（含 `{profile}.toml`）。
    #[serde(default = "default_prompts_root")]
    pub prompts_root: PathBuf,

    /// 工作目录（沙箱硬限制范围）。
    #[serde(default = "default_working_dir")]
    pub working_dir: PathBuf,

    /// 默认 profile 名。
    #[serde(default = "default_profile")]
    pub profile: String,

    /// `(name, token)` 用户对，供 `web` 子命令构造 UserStore。
    #[serde(default)]
    pub users: Vec<(String, String)>,

    /// CORS 允许的来源列表（供 `web` 子命令）。
    #[serde(default = "default_allowed_origins")]
    pub allowed_origins: Vec<String>,
}

fn default_base_url() -> String {
    "https://api.deepseek.com/v1".to_string()
}

fn default_allowed_origins() -> Vec<String> {
    vec![
        "http://localhost:8080".to_string(),
        "http://127.0.0.1:8080".to_string(),
    ]
}
fn default_model() -> String {
    "deepseek-chat".to_string()
}
fn default_prompts_root() -> PathBuf {
    PathBuf::from("prompts/profiles")
}
fn default_working_dir() -> PathBuf {
    PathBuf::from(".")
}
fn default_profile() -> String {
    "default".to_string()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            base_url: default_base_url(),
            model: default_model(),
            prompts_root: default_prompts_root(),
            working_dir: default_working_dir(),
            profile: default_profile(),
            users: Vec::new(),
            allowed_origins: default_allowed_origins(),
        }
    }
}

impl Config {
    /// 加载配置：文件 > 默认值，再叠加环境变量 api_key 覆盖。
    pub fn load() -> anyhow::Result<Config> {
        let mut cfg = load_file_or_defaults();
        let env_key = std::env::var("DEEPSEEK_API_KEY")
            .ok()
            .or_else(|| std::env::var("FORGECLAW_API_KEY").ok());
        if let Some(k) = env_key {
            cfg.api_key = k;
        }
        Ok(cfg)
    }

    /// `config init` 用的默认模板（带一个随机 token 的示例本地用户）。
    pub fn default_for_init() -> Self {
        let token = Uuid::new_v4().to_string();
        println!("生成本地用户 token: {token}");
        Self {
            users: vec![("local".to_string(), token)],
            ..Self::default()
        }
    }

    /// 保存到 `~/.forgeclaw/config.toml`（不应用环境变量覆盖，避免泄漏）。
    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path().ok_or_else(|| anyhow::anyhow!("无法确定 HOME 目录"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(&path, text)?;
        #[cfg(unix)]
        {
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(&path, perms)?;
        }
        Ok(())
    }

    /// 脱敏 api_key 用于 `config show`。
    pub fn masked_api_key(&self) -> String {
        mask_key(&self.api_key)
    }
}

/// 仅从文件加载（不应用环境变量），供 `config set` 修改磁盘配置时使用。
pub(crate) fn load_file_or_defaults() -> Config {
    let mut cfg = Config::default();
    if let Some(path) = config_path() {
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(loaded) = toml::from_str::<Config>(&text) {
                cfg = loaded;
            }
        }
    }
    cfg
}

/// `~/.forgeclaw/config.toml` 路径（无 HOME 返回 None）。
pub(crate) fn config_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    if home.is_empty() {
        return None;
    }
    Some(PathBuf::from(home).join(".forgeclaw").join("config.toml"))
}

pub(crate) fn mask_key(key: &str) -> String {
    if key.is_empty() {
        return "(未设置)".to_string();
    }
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 8 {
        return "*".repeat(chars.len());
    }
    let head: String = chars.iter().take(4).collect();
    let tail: String = chars
        .iter()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}...{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let c = Config::default();
        assert_eq!(c.base_url, "https://api.deepseek.com/v1");
        assert_eq!(c.model, "deepseek-chat");
        assert_eq!(c.profile, "default");
        assert_eq!(c.prompts_root, PathBuf::from("prompts/profiles"));
        assert_eq!(c.working_dir, PathBuf::from("."));
        assert!(c.api_key.is_empty());
        assert!(c.users.is_empty());
        assert_eq!(
            c.allowed_origins,
            vec!["http://localhost:8080", "http://127.0.0.1:8080"]
        );
    }

    #[test]
    fn masked_api_key_rules() {
        assert_eq!(mask_key(""), "(未设置)");
        assert_eq!(mask_key("short"), "*****");
        assert_eq!(mask_key("sk-abcdef123456"), "sk-a...3456");
    }

    #[test]
    fn roundtrip_toml() {
        let c = Config {
            api_key: "sk-test".into(),
            base_url: "https://api.example.com/v1".into(),
            model: "m1".into(),
            prompts_root: PathBuf::from("prompts/profiles"),
            working_dir: PathBuf::from("/tmp/w"),
            profile: "default".into(),
            users: vec![("alice".into(), "tok1".into())],
            allowed_origins: vec!["http://app.example.com".into()],
        };
        let text = toml::to_string_pretty(&c).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.api_key, "sk-test");
        assert_eq!(parsed.model, "m1");
        assert_eq!(parsed.working_dir, PathBuf::from("/tmp/w"));
        assert_eq!(parsed.users, vec![("alice".into(), "tok1".into())]);
        assert_eq!(parsed.allowed_origins, vec!["http://app.example.com"]);
    }

    #[test]
    fn partial_file_uses_serde_defaults() {
        // 只给 api_key，其余字段走 serde default 函数；users 不写则为空 vec。
        let parsed: Config = toml::from_str(r#"api_key = "k""#).unwrap();
        assert_eq!(parsed.api_key, "k");
        assert_eq!(parsed.model, "deepseek-chat");
        assert_eq!(parsed.base_url, "https://api.deepseek.com/v1");
        assert!(parsed.users.is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn save_sets_file_permissions_0600() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        std::env::set_var("HOME", &home);
        let cfg = Config {
            api_key: "sk-test".into(),
            users: vec![("local".into(), "tok".into())],
            ..Config::default()
        };
        cfg.save().unwrap();
        let path = home.join(".forgeclaw").join("config.toml");
        assert!(path.exists());
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
