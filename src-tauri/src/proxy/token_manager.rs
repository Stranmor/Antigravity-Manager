use dashmap::DashMap;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use crate::proxy::health::HealthMonitor;
use crate::proxy::rate_limit::RateLimitTracker;
use crate::proxy::sticky_config::{SchedulingMode, StickySessionConfig};

#[derive(Debug, Clone)]
struct SessionBinding {
    account_id: String,
    created_at: Instant,
}

const SESSION_TTL_SECS: u64 = 1800;

#[derive(Debug, Clone)]
pub struct ProxyToken {
    pub account_id: String,
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: i64,
    pub timestamp: i64,
    pub email: String,
    pub account_path: PathBuf,  // 账号文件路径，用于更新
    pub project_id: Option<String>,
    pub subscription_tier: Option<String>, // "FREE" | "PRO" | "ULTRA"
}

pub struct TokenManager {
    tokens: Arc<DashMap<String, ProxyToken>>,
    current_index: Arc<AtomicUsize>,
    last_used_account: Arc<tokio::sync::Mutex<Option<(String, Instant)>>>,
    data_dir: PathBuf,
    rate_limit_tracker: Arc<RateLimitTracker>,
    sticky_config: Arc<tokio::sync::RwLock<StickySessionConfig>>,
    session_accounts: Arc<DashMap<String, SessionBinding>>,
    health_monitor: Option<Arc<HealthMonitor>>,
}

impl TokenManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            tokens: Arc::new(DashMap::new()),
            current_index: Arc::new(AtomicUsize::new(0)),
            last_used_account: Arc::new(tokio::sync::Mutex::new(None)),
            data_dir,
            rate_limit_tracker: Arc::new(RateLimitTracker::new()),
            sticky_config: Arc::new(tokio::sync::RwLock::new(StickySessionConfig::default())),
            session_accounts: Arc::new(DashMap::new()),
            health_monitor: None,
        }
    }

    #[must_use]
    pub fn with_health_monitor(mut self, monitor: Arc<HealthMonitor>) -> Self {
        self.health_monitor = Some(monitor);
        self
    }
    
    pub async fn load_accounts(&self) -> Result<usize, String> {
        let accounts_dir = self.data_dir.join("accounts");
        
        if !accounts_dir.exists() {
            return Err(format!("账号目录不存在: {}", accounts_dir.display()));
        }

        // Reload should reflect current on-disk state (accounts can be added/removed/disabled).
        self.tokens.clear();
        self.current_index.store(0, Ordering::SeqCst);
        {
            let mut last_used = self.last_used_account.lock().await;
            *last_used = None;
        }
        
        let mut entries = tokio::fs::read_dir(&accounts_dir).await
            .map_err(|e| format!("读取账号目录失败: {e}"))?;

        let mut count = 0;

        while let Some(entry) = entries.next_entry().await.map_err(|e| format!("读取目录项失败: {e}"))? {
            let path = entry.path();
            
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            
            // 尝试加载账号
            match self.load_single_account(&path).await {
                Ok(Some(token)) => {
                    let account_id = token.account_id.clone();
                    self.tokens.insert(account_id, token);
                    count += 1;
                },
                Ok(None) => {
                    // 跳过无效账号
                },
                Err(e) => {
                    tracing::debug!("加载账号失败 {:?}: {}", path, e);
                }
            }
        }
        
        Ok(count)
    }
    
    /// 加载单个账号
    async fn load_single_account(&self, path: &PathBuf) -> Result<Option<ProxyToken>, String> {
        let content = tokio::fs::read_to_string(path).await
            .map_err(|e| format!("读取文件失败: {e}"))?;
        
        let account: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("解析 JSON 失败: {e}"))?;

        if account
            .get("disabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            tracing::debug!(
                "Skipping disabled account file: {:?} (email={})",
                path,
                account.get("email").and_then(|v| v.as_str()).unwrap_or("<unknown>")
            );
            return Ok(None);
        }

        // 检查主动禁用状态
        if account
            .get("proxy_disabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            tracing::debug!(
                "Skipping proxy-disabled account file: {:?} (email={})",
                path,
                account.get("email").and_then(|v| v.as_str()).unwrap_or("<unknown>")
            );
            return Ok(None);
        }

        let account_id = account["id"].as_str()
            .ok_or("缺少 id 字段")?
            .to_string();
        
        let email = account["email"].as_str()
            .ok_or("缺少 email 字段")?
            .to_string();
        
        let token_obj = account["token"].as_object()
            .ok_or("缺少 token 字段")?;
        
        let access_token = token_obj["access_token"].as_str()
            .ok_or("缺少 access_token")?
            .to_string();
        
        let refresh_token = token_obj["refresh_token"].as_str()
            .ok_or("缺少 refresh_token")?
            .to_string();
        
        let expires_in = token_obj["expires_in"].as_i64()
            .ok_or("缺少 expires_in")?;
        
        let timestamp = token_obj["expiry_timestamp"].as_i64()
            .ok_or("缺少 expiry_timestamp")?;
        
        // project_id 是可选的
        let project_id = token_obj.get("project_id")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        
        // 【新增】提取订阅等级 (subscription_tier 为 "FREE" | "PRO" | "ULTRA")
        let subscription_tier = account.get("quota")
            .and_then(|q| q.get("subscription_tier"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        
        Ok(Some(ProxyToken {
            account_id,
            access_token,
            refresh_token,
            expires_in,
            timestamp,
            email,
            account_path: path.clone(),
            project_id,
            subscription_tier,
        }))
    }
    
    /// 获取当前可用的 Token（支持粘性会话与智能调度）
    /// 参数 `quota_group` 用于区分 "claude" vs "gemini" 组
    /// 参数 `force_rotate` 为 true 时将忽略锁定，强制切换账号
    /// 参数 `session_id` 用于跨请求维持会话粘性
    ///
    /// 返回: (access_token, project_id, email, account_id)
    /// [IMPROVEMENT] 添加 account_id 到返回值，确保 rate limiting 使用正确的键
    pub async fn get_token(&self, quota_group: &str, force_rotate: bool, session_id: Option<&str>) -> Result<(String, String, String, String), String> {
        let mut tokens_snapshot: Vec<ProxyToken> = self.tokens.iter().map(|e| e.value().clone()).collect();
        let total = tokens_snapshot.len();
        if total == 0 {
            return Err("Token pool is empty".to_string());
        }

        // ===== 【优化】根据订阅等级排序 (优先级: ULTRA > PRO > FREE) =====
        // 理由: ULTRA/PRO 重置快，优先消耗；FREE 重置慢，用于兜底
        tokens_snapshot.sort_by(|a, b| {
            let tier_priority = |tier: &Option<String>| match tier.as_deref() {
                Some("ULTRA") => 0,
                Some("PRO") => 1,
                Some("FREE") => 2,
                _ => 3,
            };
            tier_priority(&a.subscription_tier).cmp(&tier_priority(&b.subscription_tier))
        });

        // 0. 读取当前调度配置
        let scheduling = self.sticky_config.read().await.clone();

        let mut attempted: HashSet<String> = HashSet::new();
        let mut last_error: Option<String> = None;

        for attempt in 0..total {
            let rotate = force_rotate || attempt > 0;

            // ===== 【核心】粘性会话与智能调度逻辑 =====
            let mut target_token: Option<ProxyToken> = None;
            
            // 模式 A: 粘性会话处理 (CacheFirst 或 Balance 且有 session_id)
            // SAFETY: We check session_id.is_some() before unwrapping
            if let Some(sid) = session_id.filter(|_| !rotate && scheduling.mode != SchedulingMode::PerformanceFirst) {
                
                // 1. 检查会话是否已绑定账号
                if let Some(binding) = self.session_accounts.get(sid).map(|v| v.clone()) {
                    let bound_account_id = &binding.account_id;
                    
                    // 1a. 检查会话 TTL（30分钟后过期）
                    if binding.created_at.elapsed().as_secs() > SESSION_TTL_SECS {
                        tracing::debug!("Session {} expired (TTL {}s exceeded), unbinding from account {}", sid, SESSION_TTL_SECS, bound_account_id);
                        self.session_accounts.remove(sid);
                        // Continue to Mode B/C for new account selection
                    } else {
                        // 1b. 检查账号健康状态 (HealthMonitor)
                        let is_unhealthy = self.health_monitor
                            .as_ref()
                            .is_some_and(|hm| !hm.is_available(bound_account_id));
                        
                        if is_unhealthy {
                            tracing::warn!("Session {} bound account {} is unhealthy, unbinding", sid, bound_account_id);
                            self.session_accounts.remove(sid);
                            // Continue to Mode B/C for new account selection
                        } else {
                            // 2. 检查绑定的账号是否限流 (使用 per-model 限流检查)
                            let reset_sec = self.rate_limit_tracker.get_remaining_wait_for_group(bound_account_id, Some(quota_group));
                            if reset_sec > 0 {
                                if scheduling.mode == SchedulingMode::CacheFirst && reset_sec <= scheduling.max_wait_seconds {
                                    // 缓存优先模式：限流时间短，执行精准精准避让等待
                                    tracing::warn!("Cache-first: Session {} bound to {} is limited. Executing precise wait for {}s to preserve cache...", sid, bound_account_id, reset_sec);
                                    tokio::time::sleep(std::time::Duration::from_secs(reset_sec)).await;
                                    
                                    // 等待后若账号可用，优先复用
                                    if let Some(found) = tokens_snapshot.iter().find(|t| &t.account_id == bound_account_id) {
                                        tracing::debug!("Sticky Session: Successfully recovered and reusing bound account {} for session {}", found.email, sid);
                                        target_token = Some(found.clone());
                                    }
                                } else {
                                    // 平衡模式或等待时间过长：断开绑定，准备换号
                                    tracing::warn!("Avoidance/WaitTimeout: Session {} switching from {} (remaining wait: {}s > limit: {}s).", sid, bound_account_id, reset_sec, scheduling.max_wait_seconds);
                                    self.session_accounts.remove(sid);
                                }
                            } else if !attempted.contains(bound_account_id) {
                                // 3. 账号可用且未被标记为尝试失败，优先复用
                                if let Some(found) = tokens_snapshot.iter().find(|t| &t.account_id == bound_account_id) {
                                    tracing::debug!("Sticky Session: Successfully reusing bound account {} for session {}", found.email, sid);
                                    target_token = Some(found.clone());
                                }
                            }
                        }
                    }
                }
            }

            // 模式 B: 原子化 60s 全局锁定 (针对无 session_id 情况的默认保护)
            if target_token.is_none() && !rotate && quota_group != "image_gen" {
                let mut last_used = self.last_used_account.lock().await;
                
                // 尝试复用全局锁定账号
                if let Some((account_id, last_time)) = &*last_used {
                    if last_time.elapsed().as_secs() < 60 && !attempted.contains(account_id) {
                        if let Some(found) = tokens_snapshot.iter().find(|t| &t.account_id == account_id) {
                            tracing::debug!("60s Window: Force reusing last account: {}", found.email);
                            target_token = Some(found.clone());
                        }
                    }
                }
                
                // 若无锁定，则轮询选择新账号
                if target_token.is_none() {
                    let start_idx = self.current_index.fetch_add(1, Ordering::SeqCst) % total;
                    for offset in 0..total {
                        let idx = (start_idx + offset) % total;
                        let candidate = &tokens_snapshot[idx];
                        if attempted.contains(&candidate.account_id) {
                            continue;
                        }

                        // 主动避开限流账号 (per-model check)
                        if self.rate_limit_tracker.is_rate_limited_for_group(&candidate.account_id, Some(quota_group)) {
                            continue;
                        }

                        // 主动避开健康监控禁用的账号
                        if let Some(ref hm) = self.health_monitor {
                            if !hm.is_available(&candidate.account_id) {
                                continue;
                            }
                        }

                        target_token = Some(candidate.clone());
                        *last_used = Some((candidate.account_id.clone(), std::time::Instant::now()));
                        
                        // 如果是会话首次分配且需要粘性，在此建立绑定
                        if let Some(sid) = session_id {
                            if scheduling.mode != SchedulingMode::PerformanceFirst {
                                self.session_accounts.insert(sid.to_string(), SessionBinding {
                                    account_id: candidate.account_id.clone(),
                                    created_at: Instant::now(),
                                });
                                tracing::debug!("Sticky Session: Bound new account {} to session {}", candidate.email, sid);
                            }
                        }
                        break;
                    }
                }
            } else if target_token.is_none() {
                // 模式 C: 纯轮询模式 (Round-robin) 或强制轮换
                let start_idx = self.current_index.fetch_add(1, Ordering::SeqCst) % total;
                for offset in 0..total {
                    let idx = (start_idx + offset) % total;
                    let candidate = &tokens_snapshot[idx];
                    if attempted.contains(&candidate.account_id) {
                        continue;
                    }

                    if self.rate_limit_tracker.is_rate_limited_for_group(&candidate.account_id, Some(quota_group)) {
                        continue;
                    }

                    if let Some(ref hm) = self.health_monitor {
                        if !hm.is_available(&candidate.account_id) {
                            continue;
                        }
                    }

                    target_token = Some(candidate.clone());
                    
                    if rotate {
                        tracing::debug!("Force Rotation: Switched to account: {}", candidate.email);
                    }
                    break;
                }
            }
            
            let Some(mut token) = target_token else {
                // 如果所有账号都被尝试过或都处于限流中，计算最短等待时间
                let min_wait = tokens_snapshot.iter()
                    .filter_map(|t| self.rate_limit_tracker.get_reset_seconds(&t.account_id))
                    .min()
                    .unwrap_or(60);

                return Err(format!("All accounts are currently limited or unhealthy. Please wait {min_wait}s."));
            };

        
            // 3. 检查 token 是否过期（提前5分钟刷新）
            let now = chrono::Utc::now().timestamp();
            if now >= token.timestamp - 300 {
                tracing::debug!("账号 {} 的 token 即将过期，正在刷新...", token.email);

                // 调用 OAuth 刷新 token
                match crate::modules::oauth::refresh_access_token(&token.refresh_token).await {
                    Ok(token_response) => {
                        tracing::debug!("Token 刷新成功！");

                        // 更新本地内存对象供后续使用
                        token.access_token.clone_from(&token_response.access_token);
                        token.expires_in = token_response.expires_in;
                        token.timestamp = now + token_response.expires_in;

                        // 同步更新跨线程共享的 DashMap
                        if let Some(mut entry) = self.tokens.get_mut(&token.account_id) {
                            entry.access_token.clone_from(&token.access_token);
                            entry.expires_in = token.expires_in;
                            entry.timestamp = token.timestamp;
                        }

                        // 同步落盘（避免重启后继续使用过期 timestamp 导致频繁刷新）
                        if let Err(e) = self.save_refreshed_token(&token.account_id, &token_response).await {
                            tracing::debug!("保存刷新后的 token 失败 ({}): {}", token.email, e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Token 刷新失败 ({}): {}，尝试下一个账号", token.email, e);
                        if e.contains("\"invalid_grant\"") || e.contains("invalid_grant") {
                            tracing::error!(
                                "Disabling account due to invalid_grant ({}): refresh_token likely revoked/expired",
                                token.email
                            );
                            let _ = self
                                .disable_account(&token.account_id, &format!("invalid_grant: {e}"))
                                .await;
                            self.tokens.remove(&token.account_id);
                        }
                        // Avoid leaking account emails to API clients; details are still in logs.
                        last_error = Some(format!("Token refresh failed: {e}"));
                        attempted.insert(token.account_id.clone());

                        // 如果当前账号被锁定复用，刷新失败后必须解除锁定，避免下一次仍选中同一账号
                        if quota_group != "image_gen" {
                            let mut last_used = self.last_used_account.lock().await;
                            if matches!(&*last_used, Some((id, _)) if id == &token.account_id) {
                                *last_used = None;
                            }
                        }
                        continue;
                    }
                }
            }

            // 4. 确保有 project_id
            let project_id = if let Some(pid) = &token.project_id {
                pid.clone()
            } else {
                tracing::debug!("账号 {} 缺少 project_id，尝试获取...", token.email);
                match crate::proxy::project_resolver::fetch_project_id(&token.access_token).await {
                    Ok(pid) => {
                        if let Some(mut entry) = self.tokens.get_mut(&token.account_id) {
                            entry.project_id = Some(pid.clone());
                        }
                        let _ = self.save_project_id(&token.account_id, &pid).await;
                        pid
                    }
                    Err(e) => {
                        tracing::error!("Failed to fetch project_id for {}: {}", token.email, e);
                        last_error = Some(format!("Failed to fetch project_id for {}: {}", token.email, e));
                        attempted.insert(token.account_id.clone());

                        if quota_group != "image_gen" {
                            let mut last_used = self.last_used_account.lock().await;
                            if matches!(&*last_used, Some((id, _)) if id == &token.account_id) {
                                *last_used = None;
                            }
                        }
                        continue;
                    }
                }
            };

            return Ok((token.access_token, project_id, token.email, token.account_id));
        }

        Err(last_error.unwrap_or_else(|| "All accounts failed".to_string()))
    }

    async fn disable_account(&self, account_id: &str, reason: &str) -> Result<(), String> {
        let path = if let Some(entry) = self.tokens.get(account_id) {
            entry.account_path.clone()
        } else {
            self.data_dir
                .join("accounts")
                .join(format!("{account_id}.json"))
        };

        let mut content: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(&path).await.map_err(|e| format!("读取文件失败: {e}"))?,
        )
        .map_err(|e| format!("解析 JSON 失败: {e}"))?;

        let now = chrono::Utc::now().timestamp();
        content["disabled"] = serde_json::Value::Bool(true);
        content["disabled_at"] = serde_json::Value::Number(now.into());
        content["disabled_reason"] = serde_json::Value::String(truncate_reason(reason, 800));

        let json_content = serde_json::to_string_pretty(&content)
            .map_err(|e| format!("Failed to serialize account data: {e}"))?;
        tokio::fs::write(&path, json_content).await
            .map_err(|e| format!("Failed to write account file: {e}"))?;

        tracing::warn!("Account disabled: {} ({:?})", account_id, path);
        Ok(())
    }

    /// 保存 project_id 到账号文件
    async fn save_project_id(&self, account_id: &str, project_id: &str) -> Result<(), String> {
        let entry = self.tokens.get(account_id)
            .ok_or("账号不存在")?;
        
        let path = &entry.account_path;
        
        let mut content: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(path).await.map_err(|e| format!("读取文件失败: {e}"))?
        ).map_err(|e| format!("解析 JSON 失败: {e}"))?;

        content["token"]["project_id"] = serde_json::Value::String(project_id.to_string());

        let json_content = serde_json::to_string_pretty(&content)
            .map_err(|e| format!("Failed to serialize project_id data: {e}"))?;
        tokio::fs::write(path, json_content).await
            .map_err(|e| format!("Failed to write account file: {e}"))?;
        
        tracing::debug!("已保存 project_id 到账号 {}", account_id);
        Ok(())
    }
    
    /// 保存刷新后的 token 到账号文件
    async fn save_refreshed_token(&self, account_id: &str, token_response: &crate::modules::oauth::TokenResponse) -> Result<(), String> {
        let entry = self.tokens.get(account_id)
            .ok_or("账号不存在")?;
        
        let path = &entry.account_path;
        
        let mut content: serde_json::Value = serde_json::from_str(
            &tokio::fs::read_to_string(path).await.map_err(|e| format!("读取文件失败: {e}"))?
        ).map_err(|e| format!("解析 JSON 失败: {e}"))?;

        let now = chrono::Utc::now().timestamp();

        content["token"]["access_token"] = serde_json::Value::String(token_response.access_token.clone());
        content["token"]["expires_in"] = serde_json::Value::Number(token_response.expires_in.into());
        content["token"]["expiry_timestamp"] = serde_json::Value::Number((now + token_response.expires_in).into());

        let json_content = serde_json::to_string_pretty(&content)
            .map_err(|e| format!("Failed to serialize token data: {e}"))?;
        tokio::fs::write(path, json_content).await
            .map_err(|e| format!("Failed to write account file: {e}"))?;
        
        tracing::debug!("已保存刷新后的 token 到账号 {}", account_id);
        Ok(())
    }
    
    pub fn len(&self) -> usize {
        self.tokens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty()
    }

    /// 获取可用账号数量 (不含限流中的账号)
    pub fn available_count(&self) -> usize {
        self.tokens
            .iter()
            .filter(|entry| !self.rate_limit_tracker.is_rate_limited(entry.key()))
            .count()
    }
    
    // ===== 限流管理方法 =====

    /// 标记账号限流(从外部调用,通常在 handler 中)
    /// 现在支持 per-model rate limiting：同一账号的 Claude 和 Gemini 配额独立跟踪
    pub fn mark_rate_limited(
        &self,
        account_id: &str,
        status: u16,
        retry_after_header: Option<&str>,
        error_body: &str,
    ) {
        self.mark_rate_limited_for_group(account_id, status, retry_after_header, error_body, None);
    }

    /// 标记账号限流（带配额组支持）
    /// quota_group: "claude", "agent", "gemini", "image_gen" 等
    pub fn mark_rate_limited_for_group(
        &self,
        account_id: &str,
        status: u16,
        retry_after_header: Option<&str>,
        error_body: &str,
        quota_group: Option<&str>,
    ) {
        self.rate_limit_tracker.parse_from_error_with_group(
            account_id,
            status,
            retry_after_header,
            error_body,
            quota_group,
        );
    }

    /// 检查账号是否在限流中
    pub fn is_rate_limited(&self, account_id: &str) -> bool {
        self.rate_limit_tracker.is_rate_limited(account_id)
    }

    /// 检查账号是否在限流中（带配额组）
    /// 允许同一账号的 Claude 和 Gemini 配额独立检查
    #[allow(dead_code)]
    pub fn is_rate_limited_for_group(&self, account_id: &str, quota_group: Option<&str>) -> bool {
        self.rate_limit_tracker.is_rate_limited_for_group(account_id, quota_group)
    }
    
    /// 获取距离限流重置还有多少秒
    #[allow(dead_code)]
    pub fn get_rate_limit_reset_seconds(&self, account_id: &str) -> Option<u64> {
        self.rate_limit_tracker.get_reset_seconds(account_id)
    }
    
    /// 清除过期的限流记录
    #[allow(dead_code)]
    pub fn cleanup_expired_rate_limits(&self) -> usize {
        self.rate_limit_tracker.cleanup_expired()
    }
    
    /// 清除指定账号的限流记录
    #[allow(dead_code)]
    pub fn clear_rate_limit(&self, account_id: &str) -> bool {
        self.rate_limit_tracker.clear(account_id)
    }

    // ===== 调度配置相关方法 =====

    /// 获取当前调度配置
    pub async fn get_sticky_config(&self) -> StickySessionConfig {
        self.sticky_config.read().await.clone()
    }

    /// 更新调度配置
    pub async fn update_sticky_config(&self, new_config: StickySessionConfig) {
        let mut config = self.sticky_config.write().await;
        *config = new_config;
        tracing::debug!("Scheduling configuration updated: {:?}", *config);
    }

    /// 清除特定会话的粘性映射
    #[allow(dead_code)]
    pub fn clear_session_binding(&self, session_id: &str) {
        self.session_accounts.remove(session_id);
    }

    /// 解绑会话从指定账号（当账号被限流时调用）
    /// 返回被解绑的会话 ID 列表
    pub fn unbind_sessions_from_account(&self, account_id: &str) -> Vec<String> {
        let mut unbound_sessions = Vec::new();

        // 遍历所有会话，找到绑定到该账号的会话并解绑
        self.session_accounts.retain(|session_id, binding| {
            if binding.account_id == account_id {
                tracing::debug!(
                    "Rate-limit unbind: Session {} detached from account {}",
                    session_id,
                    account_id
                );
                unbound_sessions.push(session_id.clone());
                false // Remove this binding
            } else {
                true // Keep this binding
            }
        });

        if !unbound_sessions.is_empty() {
            tracing::info!(
                "Unbound {} session(s) from rate-limited account {}",
                unbound_sessions.len(),
                account_id
            );
        }

        unbound_sessions
    }

    /// 标记账号限流并自动解绑所有关联会话
    /// 这是 mark_rate_limited_for_group 的增强版本
    pub fn mark_rate_limited_and_unbind(
        &self,
        account_id: &str,
        status: u16,
        retry_after_header: Option<&str>,
        error_body: &str,
        quota_group: Option<&str>,
    ) {
        // 1. 标记限流
        self.rate_limit_tracker.parse_from_error_with_group(
            account_id,
            status,
            retry_after_header,
            error_body,
            quota_group,
        );

        // 2. 自动解绑所有关联会话
        self.unbind_sessions_from_account(account_id);
    }

    /// 清除所有会话的粘性映射
    pub fn clear_all_sessions(&self) {
        self.session_accounts.clear();
    }
}

fn truncate_reason(reason: &str, max_len: usize) -> String {
    if reason.chars().count() <= max_len {
        return reason.to_string();
    }
    let mut s: String = reason.chars().take(max_len).collect();
    s.push('…');
    s
}
