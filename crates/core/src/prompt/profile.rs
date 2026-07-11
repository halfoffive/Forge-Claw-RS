//! Profile 加载：从 TOML 反序列化为 [`Profile`]。
//!
//! TOML 结构示例（`prompts/profiles/default.toml`）：
//! ```toml
//! [profile]
//! name = "default"
//! model_hint = "deepseek-chat"
//!
//! [sections]
//! identity = "sections/identity.md"
//! safety   = "sections/safety.md"
//! ```
//!
//! 不引入 `indexmap`：`sections` 用 `Vec<(String, PathBuf)>` 表达，
//! 因最终编译时按 Section 自身的 `order` 排序，TOML 插入顺序无关。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// 一个 Profile：模型提示 + 一组 section 引用。
#[derive(Debug, Clone)]
pub struct Profile {
    pub name: String,
    pub model_hint: String,
    pub sections: Vec<(String, PathBuf)>,
}

/// TOML 文件镜像结构（用于反序列化与序列化）。
#[derive(Debug, Deserialize, Serialize)]
struct ProfileFile {
    profile: ProfileMeta,
    sections: HashMap<String, PathBuf>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ProfileMeta {
    name: String,
    model_hint: String,
}

/// 从 TOML 文本解析 Profile（不读文件）。
pub fn parse_profile(text: &str) -> Result<Profile> {
    let file: ProfileFile = toml::from_str(text)?;
    Ok(Profile {
        name: file.profile.name,
        model_hint: file.profile.model_hint,
        sections: file.sections.into_iter().collect(),
    })
}

/// 从文件加载 Profile。
pub fn load_profile(path: impl AsRef<Path>) -> Result<Profile> {
    let text = std::fs::read_to_string(path.as_ref())?;
    parse_profile(&text)
}

/// 将 Profile 写回 TOML 文件。
pub fn save_profile(path: impl AsRef<Path>, profile: &Profile) -> Result<()> {
    let sections: HashMap<String, PathBuf> =
        profile.sections.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let file = ProfileFile {
        profile: ProfileMeta {
            name: profile.name.clone(),
            model_hint: profile.model_hint.clone(),
        },
        sections,
    };
    let text =
        toml::to_string_pretty(&file).map_err(|e| crate::error::CoreError::TomlSerialize(e.to_string()))?;
    std::fs::write(path.as_ref(), text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreError;

    const TOML: &str = r#"[profile]
name = "default"
model_hint = "deepseek-chat"

[sections]
identity = "sections/identity.md"
safety = "sections/safety.md"
tools = "sections/tools.md"
style = "sections/style.md"
"#;

    #[test]
    fn parses_profile_fields() {
        let p = parse_profile(TOML).unwrap();
        assert_eq!(p.name, "default");
        assert_eq!(p.model_hint, "deepseek-chat");
    }

    #[test]
    fn parses_all_section_entries() {
        let p = parse_profile(TOML).unwrap();
        assert_eq!(p.sections.len(), 4);
        let names: Vec<&str> = p.sections.iter().map(|(k, _)| k.as_str()).collect();
        for expected in ["identity", "safety", "tools", "style"] {
            assert!(names.contains(&expected), "missing section {expected}");
        }
    }

    #[test]
    fn section_paths_preserved() {
        let p = parse_profile(TOML).unwrap();
        let id = p
            .sections
            .iter()
            .find(|(k, _)| k == "identity")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(id, PathBuf::from("sections/identity.md"));
    }

    #[test]
    fn invalid_toml_errors() {
        let err = parse_profile("not toml = = =").unwrap_err();
        assert!(matches!(err, CoreError::Toml(_)));
    }

    #[test]
    fn missing_profile_section_errors() {
        let err = parse_profile("[sections]\nfoo = \"bar.md\"\n").unwrap_err();
        assert!(matches!(err, CoreError::Toml(_)));
    }
}
