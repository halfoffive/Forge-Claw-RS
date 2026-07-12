//! [`PromptEngine`]：装配 Profile + Sections，编译最终 system prompt。
//!
//! 编译流程：
//! 1. 异步读取 `{profiles_root}/{profile_name}.toml`
//! 2. 异步加载各 section 文件（路径相对 `profiles_root` 的父目录，即提示词根）
//! 3. 按 `order` 升序排序；过滤 `enabled=false`
//! 4. 拼接为 `## {title}\n{body}` 分节，节间空行分隔
//! 5. 注入变量：`{{key}}` → `vars[key]`，简单字符串 `replace`
//! 6. 按 `(profile_name, sections, vars)` 哈希缓存结果（`std::hash::DefaultHasher`）
//!
//! 缓存语义：相同输入返回缓存的拼接字符串，不重新拼接；
//! 通过 [`PromptEngine::compile_count`] 可观测实际编译次数（用于测试与诊断）。
//!
//! 并发设计：
//! - `compile` 只需要 `&self`，可在多任务间共享；
//! - cache 用 `Arc<RwLock<HashMap>>`，临界区只含 HashMap 读写；
//! - 文件 IO 在锁外通过 `tokio::fs` 完成，避免慢盘阻塞其他编译请求。

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

use tokio::fs;

use crate::error::{CoreError, Result};
use crate::model::Section;
use crate::prompt::profile::{load_profile, parse_profile};
use crate::prompt::section::{load_section_file, parse_section};

/// 提示词编译引擎。
pub struct PromptEngine {
    profiles_root: PathBuf,
    cache: Arc<RwLock<HashMap<u64, String>>>,
    compile_count: AtomicUsize,
}

impl PromptEngine {
    /// 构造引擎，传入 profiles 根目录（如 `prompts/profiles`）。
    pub fn new(profiles_root: PathBuf) -> Self {
        Self {
            profiles_root,
            cache: Arc::new(RwLock::new(HashMap::new())),
            compile_count: AtomicUsize::new(0),
        }
    }

    /// 实际编译次数（不含缓存命中）。用于测试与可观测性。
    pub fn compile_count(&self) -> usize {
        self.compile_count.load(Ordering::Relaxed)
    }

    fn profile_path(&self, profile_name: &str) -> Result<PathBuf> {
        if !is_safe_name(profile_name) {
            return Err(CoreError::InvalidName(profile_name.to_string()));
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

    /// 异步加载全部 sections（文件 IO 在锁外完成）。
    async fn load_all_sections_async(&self, profile_name: &str) -> Result<Vec<Section>> {
        let path = self.profile_path(profile_name)?;
        let text = match fs::read_to_string(&path).await {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(CoreError::ProfileNotFound(profile_name.to_string()));
            }
            Err(e) => return Err(CoreError::Io(e)),
        };
        let profile = parse_profile(&text)?;
        let base = self.sections_base();
        let mut sections = Vec::with_capacity(profile.sections.len());
        for (name, rel) in profile.sections {
            if !is_safe_name(&name) {
                return Err(CoreError::InvalidName(name));
            }
            let abs = base.join(&rel);
            let text = match fs::read_to_string(&abs).await {
                Ok(t) => t,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Err(CoreError::SectionNotFound(name, rel));
                }
                Err(e) => return Err(CoreError::Io(e)),
            };
            let section = parse_section(&text)?;
            sections.push(section);
        }
        Ok(sections)
    }

    /// 同步加载全部 sections（供 `list_sections` 使用）。
    fn load_all_sections_sync(&self, profile_name: &str) -> Result<Vec<Section>> {
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
                return Err(CoreError::InvalidName(name));
            }
            let abs = base.join(&rel);
            let section = load_section_file(&abs).map_err(|e| match e {
                CoreError::Io(ref io_err) if io_err.kind() == std::io::ErrorKind::NotFound => {
                    CoreError::SectionNotFound(name.clone(), rel)
                }
                other => other,
            })?;
            sections.push(section);
        }
        Ok(sections)
    }

    /// 列出 profile 启用的 sections（按 order 升序）。
    pub fn list_sections(&self, profile_name: &str) -> Result<Vec<Section>> {
        let sections = self.load_all_sections_sync(profile_name)?;
        Ok(enabled_sorted(sections))
    }

    /// 编译 profile 为最终 system prompt。
    ///
    /// `vars` 中常见的 key：`tools` / `model` / `cwd`，
    /// 会替换 section body 中的 `{{tools}}` / `{{model}}` / `{{cwd}}`。
    pub async fn compile(&self, profile_name: &str, vars: &HashMap<&str, String>) -> Result<String> {
        // 1. 文件 IO 在锁外完成。
        let sections = self.load_all_sections_async(profile_name).await?;
        let enabled = enabled_sorted(sections);

        let key = compute_cache_key(profile_name, &enabled, vars);

        // 2. 短读锁检查缓存。
        {
            let cache = self.cache.read().unwrap();
            if let Some(cached) = cache.get(&key) {
                return Ok(cached.clone());
            }
        }

        // 3. 在锁外拼接输出。
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

        // 4. 短写锁插入缓存（带二次检查，避免并发重复编译）。
        {
            let mut cache = self.cache.write().unwrap();
            if let Some(cached) = cache.get(&key) {
                return Ok(cached.clone());
            }
            self.compile_count.fetch_add(1, Ordering::Relaxed);
            cache.insert(key, output.clone());
        }

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
    use std::sync::Arc;

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

    #[tokio::test]
    async fn compiles_default_profile_with_all_sections() {
        let engine = PromptEngine::new(profiles_root());
        let out = engine.compile("default", &vars()).await.expect("compile failed");
        for title in ["身份与产品信息", "安全与拒绝处理", "工具使用", "语气与格式"]
        {
            assert!(
                out.contains(&format!("## {title}")),
                "output missing section `{title}`\n{out}"
            );
        }
    }

    #[tokio::test]
    async fn variables_are_replaced() {
        let engine = PromptEngine::new(profiles_root());
        let out = engine.compile("default", &vars()).await.expect("compile failed");
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

    #[tokio::test]
    async fn cache_hit_does_not_rebuild() {
        let engine = PromptEngine::new(profiles_root());
        let v = vars();
        let first = engine.compile("default", &v).await.expect("compile failed");
        let count_after_first = engine.compile_count();
        assert_eq!(count_after_first, 1, "first compile should rebuild");

        let second = engine.compile("default", &v).await.expect("compile failed");
        assert_eq!(first, second, "cached result should equal first");
        assert_eq!(
            engine.compile_count(),
            count_after_first,
            "second compile should hit cache, not rebuild"
        );
    }

    #[tokio::test]
    async fn different_vars_invalidate_cache() {
        let engine = PromptEngine::new(profiles_root());
        let v1 = vars();
        let mut v2 = vars();
        v2.insert("cwd", "/other/workspace".to_string());

        engine.compile("default", &v1).await.unwrap();
        let count1 = engine.compile_count();
        engine.compile("default", &v2).await.unwrap();
        assert_eq!(
            engine.compile_count(),
            count1 + 1,
            "different vars should cause cache miss"
        );
    }

    #[tokio::test]
    async fn concurrent_compiles_do_not_block_each_other() {
        let engine = Arc::new(PromptEngine::new(profiles_root()));
        let vars = Arc::new(vars());
        let mut handles = Vec::new();

        for _ in 0..10 {
            let engine = engine.clone();
            let vars = vars.clone();
            handles.push(tokio::spawn(async move {
                engine.compile("default", &vars).await
            }));
        }

        let mut prompts = Vec::new();
        for handle in handles {
            let prompt = handle.await.expect("task panicked").expect("compile failed");
            assert!(prompt.contains("## 身份与产品信息"));
            prompts.push(prompt);
        }

        // 所有结果应相同（共享缓存）。
        let first = prompts.first().unwrap();
        for prompt in &prompts {
            assert_eq!(prompt, first, "all concurrent compiles should return identical prompt");
        }

        // 虽然 10 个任务并发，但 key 相同，只应实际编译一次。
        assert_eq!(
            engine.compile_count(),
            1,
            "only one compile should build under concurrent identical requests"
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
                matches!(err, CoreError::InvalidName(_)),
                "expected InvalidName for {:?}, got {:?}",
                bad,
                err
            );
        }
    }

    #[tokio::test]
    async fn compile_rejects_invalid_profile_name() {
        let engine = PromptEngine::new(profiles_root());
        let err = engine.compile("../etc", &vars()).await.unwrap_err();
        assert!(
            matches!(err, CoreError::InvalidName(ref name) if name == "../etc"),
            "expected InvalidName(../etc), got {:?}",
            err
        );
    }
}
