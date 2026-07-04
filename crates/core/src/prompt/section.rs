//! Section 解析：从带 YAML frontmatter 的 Markdown 文本构造 [`Section`]。
//!
//! frontmatter 约定（位于两个 `---` 行之间，**手写解析**，不依赖 serde_yaml）：
//! ```text
//! ---
//! id: identity
//! title: "身份与产品信息"
//! level: allow          # critical | confirm | allow
//! enabled: true
//! order: 10
//! ---
//! <body>
//! ```
//!
//! 解析规则（按最简实现处理）：
//! - 首个非空行必须是 `---`；否则视为缺 frontmatter 报错
//! - 在下一个 `---` 行之间的每一行按 `key: value` 解析；忽略空行与 `#` 注释行
//! - value 自动剥离首尾 `"`；`level` 必须是 `critical|confirm|allow` 三者之一
//! - frontmatter 之后的全部内容（去掉首个空行）作为 `body`
//! - 未知字段忽略；`id`/`title`/`level` 必填，`enabled` 默认 `true`，`order` 默认 `0`

use std::path::Path;

use crate::error::{CoreError, Result};
use crate::model::{SafetyLevel, Section};

/// 从文本解析 Section。
pub fn parse_section(text: &str) -> Result<Section> {
    // 去掉 BOM，再按行切分。
    let trimmed = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = trimmed.lines().collect();

    // 首个非空行必须是 `---`。
    let mut start = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if t == "---" {
            start = Some(i);
        }
        break;
    }
    let start =
        start.ok_or_else(|| CoreError::Parse("missing frontmatter opening `---`".into()))?;

    // 找到闭合 `---`。
    let mut end = None;
    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        if line.trim() == "---" {
            end = Some(i);
            break;
        }
    }
    let end = end.ok_or_else(|| CoreError::Parse("missing frontmatter closing `---`".into()))?;

    let frontmatter = &lines[start + 1..end];
    let body = lines[end + 1..].join("\n");
    // 去掉 body 首个空行（闭合 `---` 后常紧跟一个空行）。
    let body = body.trim_start_matches('\n').to_string();

    let mut id = None;
    let mut title = None;
    let mut level = None;
    let mut enabled = true;
    let mut order = 0;

    for line in frontmatter {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        let (key, value) = t
            .split_once(':')
            .ok_or_else(|| CoreError::Parse(format!("invalid frontmatter line: {t}")))?;
        let key = key.trim();
        // 剥离首尾引号与外层空白。
        let value = value.trim().trim_matches('"');

        match key {
            "id" => id = Some(value.to_string()),
            "title" => title = Some(value.to_string()),
            "level" => {
                level = Some(match value {
                    "critical" => SafetyLevel::Critical,
                    "confirm" => SafetyLevel::Confirm,
                    "allow" => SafetyLevel::Allow,
                    _ => {
                        return Err(CoreError::Parse(format!(
                            "invalid level `{value}`, expected critical|confirm|allow"
                        )))
                    }
                });
            }
            "enabled" => {
                enabled = match value {
                    "true" => true,
                    "false" => false,
                    _ => return Err(CoreError::Parse(format!("invalid enabled `{value}`"))),
                };
            }
            "order" => {
                order = value.parse().map_err(|_| {
                    CoreError::Parse(format!("invalid order `{value}`, expected i32"))
                })?;
            }
            _ => {} // 未知字段忽略，保持前向兼容
        }
    }

    Ok(Section {
        id: id.ok_or_else(|| CoreError::Parse("frontmatter missing `id`".into()))?,
        title: title.ok_or_else(|| CoreError::Parse("frontmatter missing `title`".into()))?,
        level: level.ok_or_else(|| CoreError::Parse("frontmatter missing `level`".into()))?,
        enabled,
        order,
        body,
    })
}

/// 从文件加载 Section。
pub fn load_section_file(path: impl AsRef<Path>) -> Result<Section> {
    let text = std::fs::read_to_string(path.as_ref())?;
    parse_section(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "---\nid: identity\ntitle: \"身份与产品信息\"\nlevel: allow\nenabled: true\norder: 10\n---\n\n你是 ForgeClaw。\n";

    #[test]
    fn parses_basic_frontmatter() {
        let s = parse_section(SAMPLE).unwrap();
        assert_eq!(s.id, "identity");
        assert_eq!(s.title, "身份与产品信息");
        assert_eq!(s.level, SafetyLevel::Allow);
        assert!(s.enabled);
        assert_eq!(s.order, 10);
        assert_eq!(s.body, "你是 ForgeClaw。");
    }

    #[test]
    fn enabled_false_is_parsed() {
        let text = "---\nid: x\ntitle: t\nlevel: confirm\nenabled: false\norder: 5\n---\nbody\n";
        let s = parse_section(text).unwrap();
        assert!(!s.enabled);
        assert_eq!(s.level, SafetyLevel::Confirm);
        assert_eq!(s.order, 5);
        assert_eq!(s.body, "body");
    }

    #[test]
    fn defaults_enabled_and_order_when_absent() {
        let text = "---\nid: x\ntitle: t\nlevel: allow\n---\nbody\n";
        let s = parse_section(text).unwrap();
        assert!(s.enabled, "enabled should default to true");
        assert_eq!(s.order, 0, "order should default to 0");
    }

    #[test]
    fn level_critical_parses() {
        let text = "---\nid: x\ntitle: t\nlevel: critical\norder: 1\n---\nb\n";
        let s = parse_section(text).unwrap();
        assert_eq!(s.level, SafetyLevel::Critical);
    }

    #[test]
    fn missing_frontmatter_errors() {
        let text = "no frontmatter here\njust body";
        let err = parse_section(text).unwrap_err();
        assert!(matches!(err, CoreError::Parse(_)));
    }

    #[test]
    fn missing_closing_delimiter_errors() {
        let text = "---\nid: x\ntitle: t\nlevel: allow\nbody without close";
        let err = parse_section(text).unwrap_err();
        assert!(matches!(err, CoreError::Parse(_)));
    }

    #[test]
    fn invalid_level_errors() {
        let text = "---\nid: x\ntitle: t\nlevel: bogus\norder: 1\n---\nb\n";
        let err = parse_section(text).unwrap_err();
        assert!(matches!(err, CoreError::Parse(_)));
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let text = "---\nid: x\ntitle: t\nlevel: allow\nauthor: someone\norder: 1\n---\nb\n";
        let s = parse_section(text).unwrap();
        assert_eq!(s.id, "x");
    }

    #[test]
    fn body_keeps_internal_newlines() {
        let text = "---\nid: x\ntitle: t\nlevel: allow\norder: 1\n---\nline1\nline2\n\nline4\n";
        let s = parse_section(text).unwrap();
        assert_eq!(s.body, "line1\nline2\n\nline4");
    }

    #[test]
    fn unquoted_title_parses() {
        let text = "---\nid: x\ntitle: 裸标题\nlevel: allow\norder: 1\n---\nb\n";
        let s = parse_section(text).unwrap();
        assert_eq!(s.title, "裸标题");
    }
}
