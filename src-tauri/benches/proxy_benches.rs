use antigravity_tools_lib::models::{ProxyConfig, UpstreamProxyConfig};
use antigravity_tools_lib::proxy::mappers::openai; // Assuming this path for now, will adjust if needed
use antigravity_tools_lib::proxy::UpstreamClient;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use serde_json::json;
use std::sync::Arc;
use tokio::runtime::Runtime;

// Helper to create a dummy UpstreamClient
fn create_dummy_upstream_client() -> Arc<UpstreamClient> {
    let proxy_config = UpstreamProxyConfig {
        enabled: false,
        url: "http://localhost:8080".to_string(), // Dummy URL
    };
    Arc::new(UpstreamClient::new(Some(proxy_config)))
}

fn bench_openai_mapper(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let upstream_client = create_dummy_upstream_client();

    let mut group = c.benchmark_group("openai_mapper");

    // Benchmark for mapping a simple chat completion request
    group.bench_function("map_chat_completions_request_simple", |b| {
        let req_body = json!({
            "model": "gpt-3.5-turbo",
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });
        let proxy_config = ProxyConfig::default();

        b.to_async(&rt).iter(|| async {
            let _ = openai::map_chat_completions_request(
                black_box(req_body.clone()),
                black_box(&proxy_config),
                black_box(&upstream_client),
            )
            .await;
        });
    });

    group.finish();
}

criterion_group!(benches, bench_openai_mapper);
criterion_main!(benches);
