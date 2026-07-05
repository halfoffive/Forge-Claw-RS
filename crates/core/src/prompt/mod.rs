//! 提示词引擎：Section 解析 / Profile 加载 / 编译缓存。
//!
//! 模块组织：
//! - [`section`]：从带 YAML frontmatter 的 Markdown 文本解析 [`Section`]
//! - [`profile`]：从 TOML 加载 [`Profile`]
//! - [`engine`]：[`PromptEngine`] 装配并编译最终 system prompt
//!
//! 设计原则（karpathy CLAUDE.md）：
//! - 最小化：手写 frontmatter 解析，不引入 `serde_yaml`
//! - 缓存用 `std::hash::DefaultHasher`，不引入 `sha2`
//! - 变量注入用简单字符串 `replace`，不上模板引擎

pub mod engine;
pub mod profile;
pub mod section;

pub use engine::PromptEngine;
pub use profile::{load_profile, Profile};
pub use section::{load_section_file, parse_section};
