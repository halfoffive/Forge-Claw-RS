//! CLI 配置：从环境变量与 `~/.forgeclaw/config.toml` 加载，回退默认值。
//!
//! 优先级：环境变量 `DEEPSEEK_API_KEY` / `FORGECLAW_API_KEY` > 配置文件 > 默认值。
//! `prompts_root` 默认指向 `prompts/profiles`（`PromptEngine` 期望含 `{profile}.toml`
//! 的目录，section 路径相对其父目录解析）。

use std::path::PathBuf;

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

    /// Web CORS 白名单。空 vec 时 `web` 子命令用代码默认值（SRV-001）。
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

fn default_base_url() -> String {
    "https://api.deepseek.com/v1".to_string()
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
            allowed_origins: Vec::new(),
        }
    }
}

impl Config {
    /// 加载配置：文件 > 默认值，再叠加环境变量 api_key 覆盖，并规范化路径为绝对路径。
    pub fn load() -> anyhow::Result<Config> {
        let mut cfg = load_file_or_defaults();
        let env_key = std::env::var("DEEPSEEK_API_KEY")
            .ok()
            .or_else(|| std::env::var("FORGECLAW_API_KEY").ok());
        if let Some(k) = env_key {
            cfg.api_key = k;
        }
        cfg.prompts_root = normalize_path(&cfg.prompts_root)?;
        cfg.working_dir = normalize_path(&cfg.working_dir)?;
        Ok(cfg)
    }

    /// 早期校验：检查 api_key 是否已配置。
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.api_key.is_empty() {
            anyhow::bail!(
                "api_key 未配置：请设置环境变量 DEEPSEEK_API_KEY/FORGECLAW_API_KEY，\
                 或运行 `forgeclaw-cli config init` 后填写 ~/.forgeclaw/config.toml"
            );
        }
        Ok(())
    }

    /// `config init` 用的默认模板（带一个示例本地用户，随机 token）。
    pub fn default_for_init() -> Self {
        Self {
            users: vec![("local".to_string(), Uuid::new_v4().to_string())],
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
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(windows)]
        {
            set_windows_owner_only_dacl(&path)?;
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

/// `~/.forgeclaw/config.toml` 路径（HOME → USERPROFILE 回退，皆无返回 None）。
pub(crate) fn config_path() -> Option<PathBuf> {
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()))?;
    Some(PathBuf::from(home).join(".forgeclaw").join("config.toml"))
}

fn mask_key(key: &str) -> String {
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

fn normalize_path(p: &std::path::Path) -> anyhow::Result<PathBuf> {
    if p.is_absolute() {
        Ok(p.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(p))
    }
}

/// Windows：设置受保护 DACL，仅允许当前用户读写。
#[cfg(windows)]
fn set_windows_owner_only_dacl(path: &std::path::Path) -> anyhow::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, SET_ACCESS, SE_FILE_OBJECT,
        TRUSTEE_IS_SID, TRUSTEE_IS_USER, TRUSTEE_W,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenUser, ACL, DACL_SECURITY_INFORMATION,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_GENERIC_WRITE};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        // 打开当前进程令牌以获取用户 SID。
        let mut token: HANDLE = 0;
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            anyhow::bail!("OpenProcessToken failed: {}", GetLastError());
        }

        // 查询当前用户 SID 所需缓冲区大小。
        let mut needed: u32 = 0;
        GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed);
        if needed == 0 {
            let _ = CloseHandle(token);
            anyhow::bail!("GetTokenInformation returned zero size");
        }

        let mut user_buf = vec![0u8; needed as usize];
        if GetTokenInformation(
            token,
            TokenUser,
            user_buf.as_mut_ptr() as *mut _,
            needed,
            &mut needed,
        ) == 0
        {
            let err = GetLastError();
            let _ = CloseHandle(token);
            anyhow::bail!("GetTokenInformation failed: {}", err);
        }
        let _ = CloseHandle(token);

        let token_user = &*(user_buf.as_ptr() as *const TOKEN_USER);
        let user_sid = token_user.User.Sid;

        // 构造仅授予当前用户读写权限的显式访问项。
        let mut trustee: TRUSTEE_W = std::mem::zeroed();
        trustee.TrusteeForm = TRUSTEE_IS_SID;
        trustee.TrusteeType = TRUSTEE_IS_USER;
        trustee.ptstrName = user_sid as *mut _;

        let mut explicit_access: EXPLICIT_ACCESS_W = std::mem::zeroed();
        explicit_access.grfAccessPermissions = FILE_GENERIC_READ | FILE_GENERIC_WRITE;
        explicit_access.grfAccessMode = SET_ACCESS;
        explicit_access.grfInheritance = 0;
        explicit_access.Trustee = trustee;

        // 创建新 DACL（丢弃原有 ACE）。
        let mut acl: *mut ACL = null_mut();
        let err = SetEntriesInAclW(1, &explicit_access, null_mut(), &mut acl);
        if err != 0 {
            anyhow::bail!("SetEntriesInAclW failed: {}", err);
        }

        // 将受保护 DACL 应用到文件，阻止继承。
        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let err = SetNamedSecurityInfoW(
            path_wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null_mut(),
        );
        let _ = LocalFree(acl as *mut _);
        if err != 0 {
            anyhow::bail!("SetNamedSecurityInfoW failed: {}", err);
        }
    }
    Ok(())
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
        assert!(c.allowed_origins.is_empty());
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
            allowed_origins: vec!["http://example.com".into()],
        };
        let text = toml::to_string_pretty(&c).unwrap();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.api_key, "sk-test");
        assert_eq!(parsed.model, "m1");
        assert_eq!(parsed.working_dir, PathBuf::from("/tmp/w"));
        assert_eq!(parsed.users, vec![("alice".into(), "tok1".into())]);
        assert_eq!(
            parsed.allowed_origins,
            vec!["http://example.com".to_string()]
        );
    }

    #[test]
    fn partial_file_uses_serde_defaults() {
        // 只给 api_key，其余字段走 serde default 函数；users/allowed_origins 不写则为空 vec。
        let parsed: Config = toml::from_str(r#"api_key = "k""#).unwrap();
        assert_eq!(parsed.api_key, "k");
        assert_eq!(parsed.model, "deepseek-chat");
        assert_eq!(parsed.base_url, "https://api.deepseek.com/v1");
        assert!(parsed.users.is_empty());
        assert!(parsed.allowed_origins.is_empty());
    }

    /// Windows：验证 save() 写入的配置文件 DACL 仅包含当前用户的读写 ACE。
    #[cfg(windows)]
    #[test]
    fn save_sets_owner_only_acl_on_windows() {
        use std::os::windows::ffi::OsStrExt;
        use tempfile::tempdir;
        use windows_sys::Win32::Foundation::{GetLastError, LocalFree, PSID};
        use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
        use windows_sys::Win32::Security::{
            AclSizeInformation, EqualSid, GetAce, GetAclInformation, IsValidSid,
            ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR,
        };
        use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_GENERIC_WRITE};

        let dir = tempdir().unwrap();
        let _guard = unsafe { EnvVarGuard::set("USERPROFILE", dir.path().to_str().unwrap()) };

        let cfg = Config::default_for_init();
        cfg.save().unwrap();

        let config_file = dir.path().join(".forgeclaw").join("config.toml");
        assert!(config_file.exists());

        let user_sid = CurrentUserSid::get().unwrap();

        unsafe {
            let path_wide: Vec<u16> = config_file
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut psd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            let mut dacl: *mut ACL = std::ptr::null_mut();
            let err = GetNamedSecurityInfoW(
                path_wide.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut psd,
            );
            assert_eq!(err, 0, "GetNamedSecurityInfoW failed: {}", err);

            let mut size_info: ACL_SIZE_INFORMATION = std::mem::zeroed();
            let ok = GetAclInformation(
                dacl,
                &mut size_info as *mut _ as *mut _,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            );
            assert!(ok != 0, "GetAclInformation failed: {}", GetLastError());
            assert_eq!(size_info.AceCount, 1, "DACL should contain exactly one ACE");

            let mut ace: *mut std::ffi::c_void = std::ptr::null_mut();
            let ok = GetAce(dacl, 0, &mut ace);
            assert!(ok != 0, "GetAce failed: {}", GetLastError());

            let allowed = &*(ace as *const ACCESS_ALLOWED_ACE);
            assert_eq!(
                allowed.Header.AceType, 0,
                "ACE should be ACCESS_ALLOWED (type 0)"
            );
            assert_eq!(
                allowed.Mask,
                FILE_GENERIC_READ | FILE_GENERIC_WRITE,
                "ACE should grant read+write only, got {:#x}",
                allowed.Mask
            );

            let sid = &allowed.SidStart as *const u32 as *const std::ffi::c_void as PSID;
            assert!(IsValidSid(sid) != 0, "ACE SID is not valid");
            assert!(
                EqualSid(user_sid.sid, sid) != 0,
                "ACE SID should match current user"
            );

            let _ = LocalFree(psd);
        }
    }

    /// Windows：环境变量修改守卫，测试结束时恢复旧值。
    #[cfg(windows)]
    struct EnvVarGuard {
        key: &'static str,
        old_value: Option<String>,
    }

    #[cfg(windows)]
    impl EnvVarGuard {
        unsafe fn set(key: &'static str, value: &str) -> Self {
            let old_value = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, old_value }
        }
    }

    #[cfg(windows)]
    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.old_value {
                    Some(v) => std::env::set_var(self.key, v),
                    None => std::env::remove_var(self.key),
                }
            }
        }
    }

    /// Windows：持有当前进程令牌用户 SID 的缓冲区。
    #[cfg(windows)]
    struct CurrentUserSid {
        _buf: Vec<u8>,
        sid: windows_sys::Win32::Foundation::PSID,
    }

    #[cfg(windows)]
    impl CurrentUserSid {
        fn get() -> anyhow::Result<Self> {
            use std::ptr::null_mut;
            use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
            use windows_sys::Win32::Security::{
                GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
            };
            use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

            unsafe {
                let mut token: HANDLE = 0;
                if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                    anyhow::bail!("OpenProcessToken failed: {}", GetLastError());
                }

                let mut needed: u32 = 0;
                GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed);
                if needed == 0 {
                    let _ = CloseHandle(token);
                    anyhow::bail!("GetTokenInformation returned zero size");
                }

                let mut buf = vec![0u8; needed as usize];
                if GetTokenInformation(
                    token,
                    TokenUser,
                    buf.as_mut_ptr() as *mut _,
                    needed,
                    &mut needed,
                ) == 0
                {
                    let err = GetLastError();
                    let _ = CloseHandle(token);
                    anyhow::bail!("GetTokenInformation failed: {}", err);
                }
                let _ = CloseHandle(token);

                let token_user = &*(buf.as_ptr() as *const TOKEN_USER);
                Ok(Self {
                    sid: token_user.User.Sid,
                    _buf: buf,
                })
            }
        }
    }
}
