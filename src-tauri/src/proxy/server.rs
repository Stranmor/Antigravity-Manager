use crate::proxy::handlers;
use crate::proxy::TokenManager;
use axum::{
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Json, Response},
    routing::{any, get, post},
    Router,
};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, RwLock};
use tower_http::trace::TraceLayer;
use tracing::{debug, error, info, warn};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Server start time for uptime calculation
static SERVER_START_TIME: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Application version from Cargo.toml
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Default graceful shutdown timeout in seconds
const GRACEFUL_SHUTDOWN_TIMEOUT_SECS: u64 = 30;

/// Tracks active connections for graceful shutdown
#[derive(Debug)]
pub struct ConnectionTracker {
    /// Number of active connections being processed
    active_count: AtomicUsize,
    /// Notify when all connections are drained
    drain_complete: tokio::sync::Notify,
}

impl ConnectionTracker {
    pub fn new() -> Self {
        Self {
            active_count: AtomicUsize::new(0),
            drain_complete: tokio::sync::Notify::new(),
        }
    }

    /// Increment active connection count
    pub fn connection_started(&self) {
        self.active_count.fetch_add(1, Ordering::SeqCst);
    }

    /// Decrement active connection count and notify if drained
    pub fn connection_finished(&self) {
        let prev = self.active_count.fetch_sub(1, Ordering::SeqCst);
        if prev == 1 {
            // Was 1, now 0 - all connections drained
            self.drain_complete.notify_waiters();
        }
    }

    /// Get current active connection count
    pub fn active_connections(&self) -> usize {
        self.active_count.load(Ordering::SeqCst)
    }

    /// Wait until all connections are drained (with timeout)
    pub async fn wait_for_drain(&self, timeout: Duration) -> bool {
        let active = self.active_connections();
        if active == 0 {
            return true;
        }

        info!(
            "Waiting for {} active connection(s) to complete (timeout: {:?})",
            active, timeout
        );

        tokio::select! {
            () = self.drain_complete.notified() => {
                info!("All connections drained successfully");
                true
            }
            () = tokio::time::sleep(timeout) => {
                let remaining = self.active_connections();
                warn!(
                    "Graceful shutdown timeout reached with {} active connection(s) remaining",
                    remaining
                );
                false
            }
        }
    }
}

impl Default for ConnectionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Axum 应用状态
#[derive(Clone)]
pub struct AppState {
    pub token_manager: Arc<TokenManager>,
    pub anthropic_mapping: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
    pub openai_mapping: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
    pub custom_mapping: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
    #[allow(dead_code)]
    pub request_timeout: u64, // API 请求超时(秒)
    #[allow(dead_code)]
    pub thought_signature_map: Arc<tokio::sync::Mutex<std::collections::HashMap<String, String>>>, // 思维链签名映射 (ID -> Signature)
    #[allow(dead_code)]
    pub upstream_proxy: Arc<tokio::sync::RwLock<crate::proxy::config::UpstreamProxyConfig>>,
    pub upstream: Arc<crate::proxy::upstream::client::UpstreamClient>,
    pub zai: Arc<RwLock<crate::proxy::ZaiConfig>>,
    pub provider_rr: Arc<AtomicUsize>,
    pub zai_vision_mcp: Arc<crate::proxy::zai_vision_mcp::ZaiVisionMcpState>,
    pub monitor: Arc<crate::proxy::monitor::ProxyMonitor>,
    /// Health monitor for account health tracking with auto-disable/recovery
    pub health_monitor: Arc<crate::proxy::health::HealthMonitor>,
    /// Account-level circuit breaker for fast-fail behavior
    pub circuit_breaker: Arc<crate::proxy::common::circuit_breaker::CircuitBreakerManager>,
    /// Semantic request sampler for debugging (1% of requests by default)
    pub sampler: Arc<crate::proxy::common::sampling::RequestSampler>,
    /// Request hedger for speculative retry (tail latency optimization)
    pub hedger: Arc<crate::proxy::common::hedging::RequestHedger>,
    /// Request coalescer for deduplicating identical concurrent requests
    pub coalescer: Arc<crate::proxy::common::coalescing::CoalesceManager<serde_json::Value>>,
    /// Priority queue scheduler for fair request processing (MLQ + DRR)
    pub scheduler: Arc<crate::proxy::common::scheduler::PriorityScheduler<serde_json::Value>>,
    /// Adaptive rate limit manager (AIMD-based predictive limits)
    pub adaptive_limits: Arc<crate::proxy::adaptive_limit::AdaptiveLimitManager>,
    /// Smart prober for speculative hedging and limit discovery
    pub smart_prober: Arc<crate::proxy::smart_prober::SmartProber>,
}

/// Axum 服务器实例
pub struct AxumServer {
    /// Shutdown signal sender (broadcast for multiple receivers)
    shutdown_tx: Option<broadcast::Sender<()>>,
    anthropic_mapping: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
    openai_mapping: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
    custom_mapping: Arc<tokio::sync::RwLock<std::collections::HashMap<String, String>>>,
    proxy_state: Arc<tokio::sync::RwLock<crate::proxy::config::UpstreamProxyConfig>>,
    security_state: Arc<RwLock<crate::proxy::ProxySecurityConfig>>,
    zai_state: Arc<RwLock<crate::proxy::ZaiConfig>>,
    /// Connection tracker for graceful shutdown
    connection_tracker: Arc<ConnectionTracker>,
}

impl AxumServer {
    pub async fn update_mapping(&self, config: &crate::proxy::config::ProxyConfig) {
        {
            let mut m = self.anthropic_mapping.write().await;
            (*m).clone_from(&config.anthropic_mapping);
        }
        {
            let mut m = self.openai_mapping.write().await;
            (*m).clone_from(&config.openai_mapping);
        }
        {
            let mut m = self.custom_mapping.write().await;
            (*m).clone_from(&config.custom_mapping);
        }
        tracing::debug!("模型映射 (Anthropic/OpenAI/Custom) 已全量热更新");
    }

    /// 更新代理配置
    pub async fn update_proxy(&self, new_config: crate::proxy::config::UpstreamProxyConfig) {
        let mut proxy = self.proxy_state.write().await;
        *proxy = new_config;
        tracing::info!("上游代理配置已热更新");
    }

    pub async fn update_security(&self, config: &crate::proxy::config::ProxyConfig) {
        let mut sec = self.security_state.write().await;
        *sec = crate::proxy::ProxySecurityConfig::from_proxy_config(config);
        tracing::info!("反代服务安全配置已热更新");
    }

    pub async fn update_zai(&self, config: &crate::proxy::config::ProxyConfig) {
        let mut zai = self.zai_state.write().await;
        *zai = config.zai.clone();
        tracing::info!("z.ai 配置已热更新");
    }
    /// 启动 Axum 服务器
    #[allow(clippy::too_many_arguments)]
    pub async fn start(
        host: String,
        port: u16,
        token_manager: Arc<TokenManager>,
        anthropic_mapping: std::collections::HashMap<String, String>,
        openai_mapping: std::collections::HashMap<String, String>,
        custom_mapping: std::collections::HashMap<String, String>,
        _request_timeout: u64,
        upstream_proxy: crate::proxy::config::UpstreamProxyConfig,
        security_config: crate::proxy::ProxySecurityConfig,
        zai_config: crate::proxy::ZaiConfig,
        monitor: Arc<crate::proxy::monitor::ProxyMonitor>,

    ) -> Result<(Self, tokio::task::JoinHandle<()>), String> {
        // Initialize server start time for uptime tracking
        let _ = SERVER_START_TIME.get_or_init(Instant::now);

        let mapping_state = Arc::new(tokio::sync::RwLock::new(anthropic_mapping));
        let openai_mapping_state = Arc::new(tokio::sync::RwLock::new(openai_mapping));
        let custom_mapping_state = Arc::new(tokio::sync::RwLock::new(custom_mapping));
	        let proxy_state = Arc::new(tokio::sync::RwLock::new(upstream_proxy.clone()));
	        let security_state = Arc::new(RwLock::new(security_config));
	        let zai_state = Arc::new(RwLock::new(zai_config));
	        let provider_rr = Arc::new(AtomicUsize::new(0));
	        let zai_vision_mcp_state =
	            Arc::new(crate::proxy::zai_vision_mcp::ZaiVisionMcpState::new());

        // Initialize health monitor with default config
        let health_config = crate::proxy::health::HealthConfig::default();
        let health_monitor = crate::proxy::health::HealthMonitor::new(health_config);
        // Start the recovery background task
        let _recovery_handle = health_monitor.start_recovery_task();

        // Initialize account-level circuit breaker with default config
        let circuit_breaker_config = crate::proxy::common::circuit_breaker::CircuitBreakerConfig::default();
        let circuit_breaker = Arc::new(
            crate::proxy::common::circuit_breaker::CircuitBreakerManager::new(circuit_breaker_config)
        );

        // Initialize request sampler with default config (disabled by default)
        let sampler = Arc::new(
            crate::proxy::common::sampling::RequestSampler::new(
                crate::proxy::config::SamplingConfig::default()
            )
        );

        // Initialize request hedger with default config (disabled by default)
        let hedger = Arc::new(
            crate::proxy::common::hedging::RequestHedger::new(
                crate::proxy::config::HedgingConfig::default()
            )
        );
        // Initialize hedging metrics
        crate::proxy::common::hedging::init_hedging_metrics();

        // Initialize request coalescer with default config (disabled by default)
        let coalescer = Arc::new(
            crate::proxy::common::coalescing::CoalesceManager::new(
                crate::proxy::config::CoalescingConfig::default()
            )
        );
        // Initialize coalescing metrics
        crate::proxy::common::coalescing::init_coalescing_metrics();

        // Initialize priority queue scheduler with default config (disabled by default)
        let scheduler = Arc::new(
            crate::proxy::common::scheduler::PriorityScheduler::new(
                crate::proxy::config::SchedulerConfig::default()
            )
        );
        // Initialize scheduler metrics
        crate::proxy::common::scheduler::init_scheduler_metrics();

        // Initialize adaptive rate limit manager with default config
        let adaptive_config = crate::proxy::config::AdaptiveRateLimitConfig::default();
        let aimd = crate::proxy::adaptive_limit::AIMDController {
            additive_increase: adaptive_config.aimd_increase,
            multiplicative_decrease: adaptive_config.aimd_decrease,
            min_limit: adaptive_config.min_limit,
            max_limit: adaptive_config.max_limit,
        };
        let adaptive_limits = Arc::new(
            crate::proxy::adaptive_limit::AdaptiveLimitManager::new(
                adaptive_config.safety_margin,
                aimd,
            )
        );

        // Initialize smart prober for speculative hedging
        let prober_config = crate::proxy::smart_prober::SmartProberConfig {
            p95_latency: std::time::Duration::from_millis(adaptive_config.p95_latency_ms),
            jitter_percent: adaptive_config.jitter_percent,
            enable_cheap_probes: adaptive_config.enable_cheap_probes,
            enable_hedging: adaptive_config.enable_hedging,
        };
        let smart_prober = Arc::new(
            crate::proxy::smart_prober::SmartProber::new(prober_config, adaptive_limits.clone())
        );

	        let state = AppState {
	            token_manager: token_manager.clone(),
	            anthropic_mapping: mapping_state.clone(),
	            openai_mapping: openai_mapping_state.clone(),
	            custom_mapping: custom_mapping_state.clone(),
	            request_timeout: 300, // 5分钟超时
            thought_signature_map: Arc::new(tokio::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
            upstream_proxy: proxy_state.clone(),
            upstream: Arc::new(crate::proxy::upstream::client::UpstreamClient::new(Some(
                upstream_proxy.clone(),
            ))),
            zai: zai_state.clone(),
            provider_rr: provider_rr.clone(),
            zai_vision_mcp: zai_vision_mcp_state,
            monitor: monitor.clone(),
            health_monitor,
            circuit_breaker,
            sampler,
            hedger,
            coalescer,
            scheduler,
            adaptive_limits,
            smart_prober,
        };


        // 构建路由 - 使用新架构的 handlers！
        // 构建路由
        let app = Router::new()
            // OpenAI Protocol
            .route("/v1/models", get(handlers::openai::handle_list_models))
            .route(
                "/v1/chat/completions",
                post(handlers::openai::handle_chat_completions),
            )
            .route(
                "/v1/completions",
                post(handlers::openai::handle_completions),
            )
            .route("/v1/responses", post(handlers::openai::handle_completions)) // 兼容 Codex CLI
            .route(
                "/v1/images/generations",
                post(handlers::openai::handle_images_generations),
            ) // 图像生成 API
            .route(
                "/v1/images/edits",
                post(handlers::openai::handle_images_edits),
            ) // 图像编辑 API
            // Claude Protocol
            .route("/v1/messages", post(handlers::claude::handle_messages))
            .route(
                "/v1/messages/count_tokens",
                post(handlers::claude::handle_count_tokens),
            )
            .route(
                "/v1/models/claude",
                get(handlers::claude::handle_list_models),
            )
            // z.ai MCP (optional reverse-proxy)
            .route(
                "/mcp/web_search_prime/mcp",
                any(handlers::mcp::handle_web_search_prime),
            )
	            .route(
	                "/mcp/web_reader/mcp",
	                any(handlers::mcp::handle_web_reader),
	            )
	            .route(
	                "/mcp/zai-mcp-server/mcp",
	                any(handlers::mcp::handle_zai_mcp_server),
	            )
	            // Gemini Protocol (Native)
	            .route("/v1beta/models", get(handlers::gemini::handle_list_models))
            // Handle both GET (get info) and POST (generateContent with colon) at the same route
            .route(
                "/v1beta/models/{model}",
                get(handlers::gemini::handle_get_model).post(handlers::gemini::handle_generate),
            )
            .route(
                "/v1beta/models/{model}/countTokens",
                post(handlers::gemini::handle_count_tokens),
            ) // Specific route priority
            .route("/v1/models/detect", post(handlers::common::handle_detect_model))
            .route("/v1/api/event_logging/batch", post(silent_ok_handler))
            .route("/v1/api/event_logging", post(silent_ok_handler))
            .route("/healthz", get(health_check_handler))
            .route("/health", get(health_check_handler))
            .route("/api/health", get(health_check_handler))
            .layer(DefaultBodyLimit::max(100 * 1024 * 1024))
            .layer(axum::middleware::from_fn_with_state(state.clone(), crate::proxy::middleware::monitor::monitor_middleware))
            .layer(TraceLayer::new_for_http())
            .layer(axum::middleware::from_fn(crate::proxy::middleware::request_id_middleware))
            .layer(axum::middleware::from_fn_with_state(
                security_state.clone(),
                crate::proxy::middleware::auth_middleware,
            ))
            .layer(crate::proxy::middleware::cors_layer())
            .with_state(state);

        // 绑定地址
        let addr = format!("{host}:{port}");
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .map_err(|e| format!("地址 {addr} 绑定失败: {e}"))?;

        tracing::info!("反代服务器启动在 http://{addr}");

        // Create shutdown channel (broadcast for multiple receivers)
        // Buffer size of 1 is sufficient since we only send shutdown once
        let (shutdown_tx, _) = broadcast::channel::<()>(1);

        // Create connection tracker for graceful shutdown
        let connection_tracker = Arc::new(ConnectionTracker::new());
        let tracker_for_task = connection_tracker.clone();

        // Subscribe to shutdown signal for the accept loop
        let mut shutdown_rx = shutdown_tx.subscribe();

        let server_instance = Self {
            shutdown_tx: Some(shutdown_tx),
            anthropic_mapping: mapping_state.clone(),
            openai_mapping: openai_mapping_state.clone(),
            custom_mapping: custom_mapping_state.clone(),
            proxy_state,
            security_state,
            zai_state,
            connection_tracker,
        };

        // 在新任务中启动服务器
        let handle = tokio::spawn(async move {
            use hyper::server::conn::http1;
            use hyper_util::rt::TokioIo;
            use hyper_util::service::TowerToHyperService;

            loop {
                tokio::select! {
                    res = listener.accept() => {
                        match res {
                            Ok((stream, remote_addr)) => {
                                let io = TokioIo::new(stream);
                                let service = TowerToHyperService::new(app.clone());
                                let tracker = tracker_for_task.clone();

                                // Track this connection
                                tracker.connection_started();
                                debug!("Connection accepted from {:?} (active: {})", remote_addr, tracker.active_connections());

                                tokio::task::spawn(async move {
                                    let result = http1::Builder::new()
                                        .serve_connection(io, service)
                                        .with_upgrades() // 支持 WebSocket (如果以后需要)
                                        .await;

                                    if let Err(err) = result {
                                        // Only log if it's not a normal connection close
                                        if !err.is_incomplete_message() {
                                            debug!("连接处理结束或出错: {:?}", err);
                                        }
                                    }

                                    // Connection finished - decrement counter
                                    tracker.connection_finished();
                                    debug!("Connection closed (active: {})", tracker.active_connections());
                                });
                            }
                            Err(e) => {
                                error!("接收连接失败: {:?}", e);
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        info!("Graceful shutdown initiated - stopping new connections");
                        break;
                    }
                }
            }

            info!("Server accept loop stopped, waiting for in-flight requests to complete...");
        });

        Ok((server_instance, handle))
    }

    /// Stop the server gracefully
    ///
    /// This method:
    /// 1. Signals the server to stop accepting new connections
    /// 2. Waits for in-flight requests to complete (with 30s timeout)
    /// 3. Returns true if all connections drained cleanly, false if timeout
    pub async fn stop_gracefully(mut self) -> bool {
        info!("Initiating graceful shutdown...");

        // Signal shutdown to stop accepting new connections
        if let Some(tx) = self.shutdown_tx.take() {
            // Send to all subscribers (accept loop + any future listeners)
            let _ = tx.send(());
        }

        // Wait for active connections to drain
        let timeout = Duration::from_secs(GRACEFUL_SHUTDOWN_TIMEOUT_SECS);
        let drained = self.connection_tracker.wait_for_drain(timeout).await;

        if drained {
            info!("Graceful shutdown completed successfully");
        } else {
            warn!(
                "Graceful shutdown incomplete - {} connections still active after {}s timeout",
                self.connection_tracker.active_connections(),
                GRACEFUL_SHUTDOWN_TIMEOUT_SECS
            );
        }

        drained
    }

    /// Stop the server immediately (legacy sync method for backward compatibility)
    ///
    /// Note: This sends the shutdown signal but does NOT wait for connections to drain.
    /// For graceful shutdown, use `stop_gracefully()` instead.
    pub fn stop(mut self) {
        info!("Stopping server (immediate)...");
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        // Note: Not waiting for connections - use stop_gracefully() for that
    }

    /// Get the number of currently active connections
    pub fn active_connections(&self) -> usize {
        self.connection_tracker.active_connections()
    }
}

// ===== API 处理器 (旧代码已移除，由 src/proxy/handlers/* 接管) =====

/// 健康检查处理器 (增强版)
/// 返回详细的服务状态信息，无需认证
async fn health_check_handler(State(state): State<AppState>) -> Response {
    let start_time = SERVER_START_TIME.get_or_init(Instant::now);
    let uptime_seconds = start_time.elapsed().as_secs();

    let accounts_total = state.token_manager.len();
    let accounts_available = state.token_manager.available_count();

    // Determine health status based on account availability
    let status = if accounts_total == 0 {
        "unhealthy"
    } else if accounts_available == 0 || accounts_available < accounts_total / 2 {
        "degraded"
    } else {
        "ok"
    };

    Json(serde_json::json!({
        "status": status,
        "accounts_total": accounts_total,
        "accounts_available": accounts_available,
        "uptime_seconds": uptime_seconds,
        "version": APP_VERSION
    }))
    .into_response()
}

/// 静默成功处理器 (用于拦截遥测日志等)
async fn silent_ok_handler() -> Response {
    StatusCode::OK.into_response()
}
