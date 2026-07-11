//! 核心错误类型与 `Result` 别名。
//!
//! 所有 IO、解析与提示词装配错误统一归入 [`CoreError`]；各子模块返回 [`Result<T>`]。

use std::path::PathBuf;

/// ForgeClaw core 错误枚举。
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// 文件系统 IO 错误。
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// TOML 反序列化错误。
    #[error("toml deserialize error: {0}")]
    Toml(#[from] toml::de::Error),

    /// TOML 序列化错误。
    #[error("toml serialize error: {0}")]
    TomlSerialize(String),

    /// 通用解析错误（frontmatter 字段缺失、非法值等）。
    #[error("parse error: {0}")]
    Parse(String),

    /// 找不到对应 profile（`{name}.toml` 不存在）。
    #[error("profile not found: {0}")]
    ProfileNotFound(String),

    /// profile 引用的 section 文件不存在。
    #[error("section not found: {0} (path: {1})")]
    SectionNotFound(String, PathBuf),
}

/// core crate 统一 Result 别名。
pub type Result<T> = std::result::Result<T, CoreError>;
