// 模型名称映射
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
use lru::LruCache;
use parking_lot::Mutex;

/// LRU cache for resolved model routes (thread-safe)
/// Cache key is a hash of (original_model, custom_mapping_hash, openai_mapping_hash, anthropic_mapping_hash, apply_claude_family_mapping)
static MODEL_ROUTE_CACHE: std::sync::LazyLock<Mutex<LruCache<u64, String>>> = std::sync::LazyLock::new(|| {
    Mutex::new(LruCache::new(
        NonZeroUsize::new(100).expect("MODEL_ROUTE_CACHE: Cache size must be non-zero")
    ))
});

/// Compute a deterministic hash for a HashMap to use as part of cache key
fn hash_mapping(mapping: &HashMap<String, String>) -> u64 {
    let mut hasher = DefaultHasher::new();
    // Sort keys for deterministic hashing
    let mut keys: Vec<_> = mapping.keys().collect();
    keys.sort();
    for key in keys {
        key.hash(&mut hasher);
        if let Some(value) = mapping.get(key) {
            value.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// Compute composite cache key from all inputs
fn compute_cache_key(
    original_model: &str,
    custom_mapping: &HashMap<String, String>,
    openai_mapping: &HashMap<String, String>,
    anthropic_mapping: &HashMap<String, String>,
    apply_claude_family_mapping: bool,
) -> u64 {
    let mut hasher = DefaultHasher::new();
    original_model.hash(&mut hasher);
    hash_mapping(custom_mapping).hash(&mut hasher);
    hash_mapping(openai_mapping).hash(&mut hasher);
    hash_mapping(anthropic_mapping).hash(&mut hasher);
    apply_claude_family_mapping.hash(&mut hasher);
    hasher.finish()
}

/// Invalidate the model route cache (call when mappings are updated)
#[allow(dead_code)] // Public API for cache invalidation
pub fn invalidate_model_cache() {
    MODEL_ROUTE_CACHE.lock().clear();
    crate::modules::logger::log_info("[Cache] Model route cache invalidated");
}

static CLAUDE_TO_GEMINI: std::sync::LazyLock<HashMap<&'static str, &'static str>> = std::sync::LazyLock::new(|| {
    let mut m = HashMap::new();

    // 直接支持的模型
    m.insert("claude-opus-4-5-thinking", "claude-opus-4-5-thinking");
    m.insert("claude-sonnet-4-5", "claude-sonnet-4-5");
    m.insert("claude-sonnet-4-5-thinking", "claude-sonnet-4-5-thinking");

    // 别名映射
    m.insert("claude-sonnet-4-5-20250929", "claude-sonnet-4-5-thinking");
    m.insert("claude-3-5-sonnet-20241022", "claude-sonnet-4-5");
    m.insert("claude-3-5-sonnet-20240620", "claude-sonnet-4-5");
    m.insert("claude-opus-4", "claude-opus-4-5-thinking");
    m.insert("claude-opus-4-5-20251101", "claude-opus-4-5-thinking");
    m.insert("claude-haiku-4", "claude-sonnet-4-5");
    m.insert("claude-3-haiku-20240307", "claude-sonnet-4-5");
    m.insert("claude-haiku-4-5-20251001", "claude-sonnet-4-5");
    // OpenAI 协议映射表
    m.insert("gpt-4", "gemini-2.5-pro");
    m.insert("gpt-4-turbo", "gemini-2.5-pro");
    m.insert("gpt-4-turbo-preview", "gemini-2.5-pro");
    m.insert("gpt-4-0125-preview", "gemini-2.5-pro");
    m.insert("gpt-4-1106-preview", "gemini-2.5-pro");
    m.insert("gpt-4-0613", "gemini-2.5-pro");

    m.insert("gpt-4o", "gemini-2.5-pro");
    m.insert("gpt-4o-2024-05-13", "gemini-2.5-pro");
    m.insert("gpt-4o-2024-08-06", "gemini-2.5-pro");

    m.insert("gpt-4o-mini", "gemini-2.5-flash");
    m.insert("gpt-4o-mini-2024-07-18", "gemini-2.5-flash");

    m.insert("gpt-3.5-turbo", "gemini-2.5-flash");
    m.insert("gpt-3.5-turbo-16k", "gemini-2.5-flash");
    m.insert("gpt-3.5-turbo-0125", "gemini-2.5-flash");
    m.insert("gpt-3.5-turbo-1106", "gemini-2.5-flash");
    m.insert("gpt-3.5-turbo-0613", "gemini-2.5-flash");

    // Gemini 协议映射表
    m.insert("gemini-2.5-flash-lite", "gemini-2.5-flash-lite");
    m.insert("gemini-2.5-flash-thinking", "gemini-2.5-flash-thinking");
    m.insert("gemini-3-pro-low", "gemini-3-pro-low");
    m.insert("gemini-3-pro-high", "gemini-3-pro-high");
    m.insert("gemini-3-pro-preview", "gemini-3-pro-preview");
    m.insert("gemini-2.5-flash", "gemini-2.5-flash");
    m.insert("gemini-3-flash", "gemini-3-flash");
    m.insert("gemini-3-pro-image", "gemini-3-pro-image");

    m
});

pub fn map_claude_model_to_gemini(input: &str) -> String {
    // 1. Check exact match in map
    if let Some(mapped) = CLAUDE_TO_GEMINI.get(input) {
        return mapped.to_string();
    }

    // 2. Pass-through known prefixes (gemini-, -thinking) to support dynamic suffixes
    if input.starts_with("gemini-") || input.contains("thinking") {
        return input.to_string();
    }

    // 3. Fallback to default
    "claude-sonnet-4-5".to_string()
}

/// 获取所有内置支持的模型列表关键字
pub fn get_supported_models() -> Vec<String> {
    CLAUDE_TO_GEMINI.keys().map(std::string::ToString::to_string).collect()
}

/// 动态获取所有可用模型列表 (包含内置与用户自定义)
#[allow(clippy::implicit_hasher)]
pub async fn get_all_dynamic_models(
    openai_mapping: &tokio::sync::RwLock<std::collections::HashMap<String, String>>,
    custom_mapping: &tokio::sync::RwLock<std::collections::HashMap<String, String>>,
    anthropic_mapping: &tokio::sync::RwLock<std::collections::HashMap<String, String>>,
) -> Vec<String> {
    use std::collections::HashSet;
    let mut model_ids = HashSet::new();

    // 1. 获取所有内置映射模型
    for m in get_supported_models() {
        model_ids.insert(m);
    }

    // 2. 获取所有自定义映射模型 (OpenAI)
    {
        let mapping = openai_mapping.read().await;
        for key in mapping.keys() {
            if !key.ends_with("-series") {
                 model_ids.insert(key.clone());
            }
        }
    }

    // 3. 获取所有自定义映射模型 (Custom)
    {
        let mapping = custom_mapping.read().await;
        for key in mapping.keys() {
            model_ids.insert(key.clone());
        }
    }

    // 4. 获取所有 Anthropic 映射模型
    {
        let mapping = anthropic_mapping.read().await;
        for key in mapping.keys() {
            if !key.ends_with("-series") && key != "claude-default" {
                model_ids.insert(key.clone());
            }
        }
    }

    // 5. 确保包含常用的 Gemini/画画模型 ID
    model_ids.insert("gemini-3-pro-low".to_string());
    
    // [NEW] Issue #247: Dynamically generate all Image Gen Combinations
    let base = "gemini-3-pro-image";
    let resolutions = vec!["", "-2k", "-4k"];
    let ratios = ["", "-1x1", "-4x3", "-3x4", "-16x9", "-9x16", "-21x9"];
    
    for res in resolutions {
        for ratio in &ratios {
            let mut id = base.to_string();
            id.push_str(res);
            id.push_str(ratio);
            model_ids.insert(id);
        }
    }

    model_ids.insert("gemini-2.0-flash-exp".to_string());
    model_ids.insert("gemini-2.5-flash".to_string());
    model_ids.insert("gemini-2.5-pro".to_string());
    model_ids.insert("gemini-3-flash".to_string());
    model_ids.insert("gemini-3-pro-high".to_string());
    model_ids.insert("gemini-3-pro-low".to_string());


    let mut sorted_ids: Vec<_> = model_ids.into_iter().collect();
    sorted_ids.sort();
    sorted_ids
}

/// 核心模型路由解析引擎
/// 优先级：Custom Mapping (精确) > Group Mapping (家族) > System Mapping (内置插件)
///
/// # 参数
/// - `apply_claude_family_mapping`: 是否对 Claude 模型应用家族映射
///   - `true`: CLI 请求，应用家族映射（如 claude-sonnet-4-5 -> gemini-3-pro-high）
///   - `false`: 非 CLI 请求（如 Cherry Studio），跳过家族映射，直接穿透
///
/// # Performance
/// Uses LRU cache (100 entries) for resolved routes to avoid redundant HashMap lookups.
#[allow(clippy::implicit_hasher)]
pub fn resolve_model_route(
    original_model: &str,
    custom_mapping: &std::collections::HashMap<String, String>,
    openai_mapping: &std::collections::HashMap<String, String>,
    anthropic_mapping: &std::collections::HashMap<String, String>,
    apply_claude_family_mapping: bool,
) -> String {
    // Compute cache key from all inputs
    let cache_key = compute_cache_key(
        original_model,
        custom_mapping,
        openai_mapping,
        anthropic_mapping,
        apply_claude_family_mapping,
    );

    // Check cache first (fast path)
    {
        let mut cache = MODEL_ROUTE_CACHE.lock();
        if let Some(cached_result) = cache.get(&cache_key) {
            return cached_result.clone();
        }
    }

    // Cache miss: compute the route
    let result = resolve_model_route_uncached(
        original_model,
        custom_mapping,
        openai_mapping,
        anthropic_mapping,
        apply_claude_family_mapping,
    );

    // Store in cache
    {
        let mut cache = MODEL_ROUTE_CACHE.lock();
        cache.put(cache_key, result.clone());
    }

    result
}

/// Internal uncached implementation of model route resolution
#[allow(clippy::implicit_hasher)]
fn resolve_model_route_uncached(
    original_model: &str,
    custom_mapping: &std::collections::HashMap<String, String>,
    openai_mapping: &std::collections::HashMap<String, String>,
    anthropic_mapping: &std::collections::HashMap<String, String>,
    apply_claude_family_mapping: bool,
) -> String {
    // 1. 检查自定义精确映射 (优先级最高)
    if let Some(target) = custom_mapping.get(original_model) {
        crate::modules::logger::log_info(&format!("[Router] 使用自定义精确映射: {original_model} -> {target}"));
        return target.clone();
    }

    let lower_model = original_model.to_lowercase();

    // 2. 检查家族分组映射 (OpenAI 系)
    // GPT-4 系列 (含 GPT-4 经典, o1, o3 等, 排除 4o/mini/turbo)
    if (lower_model.starts_with("gpt-4") && !lower_model.contains('o') && !lower_model.contains("mini") && !lower_model.contains("turbo")) || 
       lower_model.starts_with("o1-") || lower_model.starts_with("o3-") || lower_model == "gpt-4" {
        if let Some(target) = openai_mapping.get("gpt-4-series") {
            crate::modules::logger::log_info(&format!("[Router] 使用 GPT-4 系列映射: {original_model} -> {target}"));
            return target.clone();
        }
    }
    
    // GPT-4o / 3.5 系列 (均衡与轻量, 含 4o, mini, turbo)
    if lower_model.contains("4o") || lower_model.starts_with("gpt-3.5") || (lower_model.contains("mini") && !lower_model.contains("gemini")) || lower_model.contains("turbo") {
        if let Some(target) = openai_mapping.get("gpt-4o-series") {
            crate::modules::logger::log_info(&format!("[Router] 使用 GPT-4o/3.5 系列映射: {original_model} -> {target}"));
            return target.clone();
        }
    }

    // GPT-5 系列 (gpt-5, gpt-5.1, gpt-5.2 等)
    if lower_model.starts_with("gpt-5") {
        // 优先使用 gpt-5-series 映射，如果没有则使用 gpt-4-series
        if let Some(target) = openai_mapping.get("gpt-5-series") {
            crate::modules::logger::log_info(&format!("[Router] 使用 GPT-5 系列映射: {original_model} -> {target}"));
            return target.clone();
        }
        if let Some(target) = openai_mapping.get("gpt-4-series") {
            crate::modules::logger::log_info(&format!("[Router] 使用 GPT-4 系列映射 (GPT-5 fallback): {original_model} -> {target}"));
            return target.clone();
        }
    }

    // 3. 检查家族分组映射 (Anthropic 系)
    if lower_model.starts_with("claude-") {
        // [CRITICAL] 检查是否应用 Claude 家族映射
        // 如果是非 CLI 请求（如 Cherry Studio），先检查是否为原生支持的直通模型
        if !apply_claude_family_mapping {
            if let Some(mapped) = CLAUDE_TO_GEMINI.get(original_model) {
                if *mapped == original_model {
                    // 原生支持的直通模型，跳过家族映射
                    crate::modules::logger::log_info(&format!("[Router] 非 CLI 请求，跳过家族映射: {original_model}"));
                    return original_model.to_string();
                }
            }
        }
        
        // [NEW] Haiku 智能降级策略
        // 将所有 Haiku 模型自动降级到 gemini-3-flash (最新的 Flash 模型)
        // [FIX] 仅在 CLI 模式下生效 (apply_claude_family_mapping == true)
        if apply_claude_family_mapping && lower_model.contains("haiku") {
            crate::modules::logger::log_info(&format!("[Router] Haiku 智能降级 (CLI): {original_model} -> gemini-3-flash"));
            return "gemini-3-flash".to_string();
        }

        let family_key = if lower_model.contains("4-5") || lower_model.contains("4.5") {
            "claude-4.5-series"
        } else if lower_model.contains("3-5") || lower_model.contains("3.5") {
            "claude-3.5-series"
        } else {
            "claude-default"
        };

        if let Some(target) = anthropic_mapping.get(family_key) {
            crate::modules::logger::log_warn(&format!("[Router] 使用 Anthropic 系列映射: {original_model} -> {target}"));
            return target.clone();
        }
        
        // 兜底兼容旧版精确映射
        if let Some(target) = anthropic_mapping.get(original_model) {
             return target.clone();
        }
    }

    // 4. 下沉到系统默认映射逻辑
    map_claude_model_to_gemini(original_model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_mapping() {
        assert_eq!(
            map_claude_model_to_gemini("claude-3-5-sonnet-20241022"),
            "claude-sonnet-4-5"
        );
        assert_eq!(
            map_claude_model_to_gemini("claude-opus-4"),
            "claude-opus-4-5-thinking"
        );
        // Test gemini pass-through (should not be caught by "mini" rule)
        assert_eq!(
            map_claude_model_to_gemini("gemini-2.5-flash-mini-test"),
            "gemini-2.5-flash-mini-test"
        );
        assert_eq!(
            map_claude_model_to_gemini("unknown-model"),
            "claude-sonnet-4-5"
        );
    }

    #[test]
    fn test_resolve_model_route_caching() {
        use std::collections::HashMap;

        let custom_mapping = HashMap::new();
        let openai_mapping = HashMap::new();
        let anthropic_mapping = HashMap::new();

        // First call - cache miss, computes route
        let result1 = resolve_model_route(
            "gpt-4",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            true,
        );

        // Second call - should hit cache
        let result2 = resolve_model_route(
            "gpt-4",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            true,
        );

        assert_eq!(result1, result2);
        assert_eq!(result1, "gemini-2.5-pro"); // GPT-4 maps to gemini-2.5-pro
    }

    #[test]
    fn test_cache_invalidation() {
        use std::collections::HashMap;

        let custom_mapping = HashMap::new();
        let openai_mapping = HashMap::new();
        let anthropic_mapping = HashMap::new();

        // Populate cache
        let _ = resolve_model_route(
            "test-model",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            false,
        );

        // Invalidate cache
        invalidate_model_cache();

        // Cache should be empty, next call will recompute
        let result = resolve_model_route(
            "test-model",
            &custom_mapping,
            &openai_mapping,
            &anthropic_mapping,
            false,
        );

        // Should still return correct result after invalidation
        assert_eq!(result, "claude-sonnet-4-5");
    }
}
