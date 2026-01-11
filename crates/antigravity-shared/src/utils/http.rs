use serde::{Deserialize, Serialize};

/// Upstream proxy configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct UpstreamProxyConfig {
    pub enabled: bool,
    pub url: String,
}
