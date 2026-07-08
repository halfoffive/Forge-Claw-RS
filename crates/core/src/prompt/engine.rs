//! [`PromptEngine`]：装配 Profile + Sections，编译最终 system prompt。
//!
//! 编译流程：
//! 1. 读取 `{profiles_root}/{profile_name}.toml`
//! 2. 加载各 section 文件（路径相对 `profiles_root` 的父目录，即提示词根）
//! 3. 按 `order` 升序排序；过滤 `enabled=false`
//! 4. 拼接为 `## {title}\n{body}` 分节，节间空行分隔
//! 5. 注入变量：`{{key}}` → `vars[key]`，简单字符串 `replace`
//! 6. 按 `(profile_name, sections, vars)` 哈希缓存结果（`std::hash::DefaultHasher`）
//!
//! 缓存语义：相同输入返回缓存的拼接字符串，不重新拼接；
//! 通过 [`PromptEngine::compile_count`] 可观测实际编译次数（用于测试与诊断）。

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

use crate::error::{CoreError, Result};
use crate::model::Section;
use crate::prompt::profile::load_profile;
use crate::prompt::section::load_section_file;

/// 提示词编译引擎。
pub struct PromptEngine {
    profiles_root: PathBuf,
    cache: HashMap<u64, String>,
    compile_count: usize,
}

impl PromptEngine {
    /// 构造引擎，传入 profiles 根目录（如 `prompts/profiles`）。
    pub fn new(profiles_root: PathBuf) -> Self {
        Self {
            profiles_root,
            cache: HashMap::new(),
            compile_count: 0,
        }
    }

    /// 实际编译次数（不含缓存命中）。用于测试与可观测性。
    pub fn compile_count(&self) -> usize {
        self.compile_count
    }

    fn profile_path(&self, profile_name: &str) -> Result<PathBuf> {
        if !is_safe_name(profile_name) {
            return Err(CoreError::ProfileNotFound(profile_name.to_string()));
        }
        Ok(self.profiles_root.join(format!("{profile_name}.toml")))
    }

    /// section 路径相对提示词根（profiles_root 的父目录）解析。
    fn sections_base(&self) -> PathBuf {
        match self.profiles_root.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => self.profiles_root.clone(),
        }
    }

    fn load_all_sections(&self, profile_name: &str) -> Result<Vec<Section>> {
        let path = self.profile_path(profile_name)?;
        let profile = load_profile(&path).map_err(|e| match e {
            CoreError::Io(ref io_err) if io_err.kind() == std::io::ErrorKind::NotFound => {
                CoreError::ProfileNotFound(profile_name.to_string())
            }
            other => other,
        })?;
        let base = self.sections_base();
        let mut sections = Vec::with_capacity(profile.sections.len());
        for (name, rel) in profile.sections {
            if !is_safe_name(&name) {
                return Err(CoreError::SectionNotFound(name, rel));
            }
            let abs = base.join(&rel);
            let section = load_section_file(&abs).map_err(|e| match e {
                CoreError::Io(ref io_err) if io_err.kind() == std::io::ErrorKind::NotFound => {
                    CoreError::SectionNotFound(name, rel)
                }
                other => other,
            })?;
            sections.push(section);
        }
        Ok(sections)
    }

    /// 列出 profile 启用的 sections（按 order 升序）。
    pub fn list_sections(&self, profile_name: &str) -> Result<Vec<Section>> {
        let sections = self.load_all_sections(profile_name)?;
        Ok(enabled_sorted(sections))
    }

    /// 编译 profile 为最终 system prompt。
    ///
    /// `vars` 中常见的 key：`tools` / `model` / `cwd`，
    /// 会替换 section body 中的 `{{tools}}` / `{{model}}` / `{{cwd}}`。
    pub fn compile(&mut self, profile_name: &str, vars: &HashMap<&str, String>) -> Result<String> {
        let sections = self.load_all_sections(profile_name)?;
        let enabled = enabled_sorted(sections);

        let key = compute_cache_key(profile_name, &enabled, vars);
        if let Some(cached) = self.cache.get(&key) {
            return Ok(cached.clone());
        }

        self.compile_count += 1;

        let mut output = String::new();
        for section in &enabled {
            if !output.is_empty() {
                output.push_str("\n\n");
            }
            output.push_str("## ");
            output.push_str(&section.title);
            output.push('\n');
            output.push_str(&section.body);
        }

        for (k, v) in vars {
            output = output.replace(&format!("{{{{{k}}}}}"), v);
        }

        self.cache.insert(key, output.clone());
        Ok(output)
    }
}

/// 校验名字仅含 `[A-Za-z0-9_-]`：拒绝 `..` / `/` / `\` / `\0` 等路径遍历字符。
fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('\0')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 按 `order` 升序排序，相同 order 时以 `id` 作 tiebreaker；过滤 `enabled=false`。
fn enabled_sorted(mut sections: Vec<Section>) -> Vec<Section> {
    sections.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    sections.retain(|s| s.enabled);
    sections
}

/// 计算缓存 key：profile 名 + 各启用 section 内容 + 排序后的 vars。
fn compute_cache_key(
    profile_name: &str,
    sections: &[Section],
    vars: &HashMap<&str, String>,
) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    profile_name.hash(&mut hasher);
    for s in sections {
        s.id.hash(&mut hasher);
        s.title.hash(&mut hasher);
        s.body.hash(&mut hasher);
        s.order.hash(&mut hasher);
        s.enabled.hash(&mut hasher);
    }
    let mut keys: Vec<&str> = vars.keys().copied().collect();
    keys.sort_unstable();
    for k in keys {
        k.hash(&mut hasher);
        vars[k].hash(&mut hasher);
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 找到仓库根的 prompts/profiles 目录。
    fn profiles_root() -> PathBuf {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        PathBuf::from(manifest_dir).join("../../prompts/profiles")
    }

    fn vars() -> HashMap<&'static str, String> {
        let mut v: HashMap<&str, String> = HashMap::new();
        v.insert("tools", "ShellTool,FileReadTool".to_string());
        v.insert("model", "deepseek-chat".to_string());
        v.insert("cwd", "/workspace".to_string());
        v
    }

    #[test]
    fn compiles_default_profile_with_all_sections() {
        let mut engine = PromptEngine::new(profiles_root());
        let out = engine.compile("default", &vars()).expect("compile failed");
        for title in ["身份与产品信息", "安全与拒绝处理", "工具使用", "语气与格式"]
        {
            assert!(
                out.contains(&format!("## {title}")),
                "output missing section `{title}`\n{out}"
            );
        }
    }

    #[test]
    fn variables_are_replaced() {
        let mut engine = PromptEngine::new(profiles_root());
        let out = engine.compile("default", &vars()).expect("compile failed");
        assert!(
            !out.contains("{{tools}}"),
            "tools variable not replaced\n{out}"
        );
        assert!(
            !out.contains("{{model}}"),
            "model variable not replaced\n{out}"
        );
        assert!(!out.contains("{{cwd}}"), "cwd variable not replaced\n{out}");
        assert!(out.contains("deepseek-chat"), "model value missing");
        assert!(out.contains("/workspace"), "cwd value missing");
        assert!(out.contains("ShellTool"), "tools value missing");
    }

    #[test]
    fn cache_hit_does_not_rebuild() {
        let mut engine = PromptEngine::new(profiles_root());
        let v = vars();
        let first = engine.compile("default", &v).expect("compile failed");
        let count_after_first = engine.compile_count();
        assert_eq!(count_after_first, 1, "first compile should rebuild");

        let second = engine.compile("default", &v).expect("compile failed");
        assert_eq!(first, second, "cached result should equal first");
        assert_eq!(
            engine.compile_count(),
            count_after_first,
            "second compile should hit cache, not rebuild"
        );
    }

    #[test]
    fn different_vars_invalidate_cache() {
        let mut engine = PromptEngine::new(profiles_root());
        let v1 = vars();
        let mut v2 = vars();
        v2.insert("cwd", "/other/workspace".to_string());

        engine.compile("default", &v1).unwrap();
        let count1 = engine.compile_count();
        engine.compile("default", &v2).unwrap();
        assert_eq!(
            engine.compile_count(),
            count1 + 1,
            "different vars should cause cache miss"
        );
    }

    #[test]
    fn list_sections_returns_enabled_sorted() {
        let engine = PromptEngine::new(profiles_root());
        let sections = engine.list_sections("default").expect("list failed");
        assert!(!sections.is_empty(), "should list at least one section");
        // 验证 order 升序
        let mut prev = i32::MIN;
        for s in &sections {
            assert!(
                s.enabled,
                "list_sections should only return enabled sections"
            );
            assert!(s.order >= prev, "sections not sorted by order");
            prev = s.order;
        }
    }

    #[test]
    fn missing_profile_errors() {
        let engine = PromptEngine::new(profiles_root());
        let err = engine.list_sections("does-not-exist").unwrap_err();
        assert!(matches!(err, CoreError::ProfileNotFound(_)));
    }

    #[test]
    fn profile_name_traversal_rejected() {
        let engine = PromptEngine::new(profiles_root());
        for bad in ["../etc", "a/b", "a\\b", "..", "a\x00b", "a.b"] {
            let err = engine.list_sections(bad).unwrap_err();
            assert!(
                matches!(err, CoreError::ProfileNotFound(_)),
                "expected ProfileNotFound for {:?}, got {:?}",
                bad,
                err
            );
        }
    }
}
