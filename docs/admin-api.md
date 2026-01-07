# Admin API Documentation

The Antigravity Server exposes a REST API for remote management and monitoring.

## Base URL

- **Local:** `http://localhost:9101`
- **Production:** `https://antigravity.quantumind.ru` (proxied via nginx)

## Authentication

All endpoints (except `/api/health`) require Bearer token authentication:

```bash
curl -H "Authorization: Bearer YOUR_API_KEY" https://antigravity.quantumind.ru/api/accounts
```

Set the API key via environment variable:
```bash
ANTIGRAVITY_API_KEY=sk-ag-your-secret-key
```

## Endpoints

### Health Check

#### `GET /api/health`
Basic health check (no auth required).

**Response:**
```json
{
  "status": "ok",
  "version": "3.3.15",
  "uptime_seconds": 3600,
  "accounts_total": 6,
  "accounts_available": 5,
  "proxy_running": true,
  "proxy_port": 8045
}
```

#### `GET /api/health/detailed`
Detailed component health status.

**Response:**
```json
{
  "status": "healthy",
  "version": "3.3.15",
  "uptime_seconds": 3600,
  "components": {
    "database": { "status": "healthy", "latency_ms": 2 },
    "token_manager": { "status": "healthy", "accounts_total": 6, "accounts_available": 5 },
    "circuit_breaker": { "status": "healthy", "closed": 6, "open": 0, "half_open": 0 },
    "proxy_server": { "status": "healthy" },
    "log_rotation": { "status": "healthy", "size_bytes": 1048576 }
  },
  "checks": {
    "disk_space_ok": true,
    "memory_ok": true,
    "cpu_ok": true
  },
  "timestamp": "2026-01-07T12:00:00Z"
}
```

### Account Management

#### `GET /api/accounts`
List all registered accounts.

**Response:**
```json
[
  {
    "id": "abc123",
    "email": "user@gmail.com",
    "tier": "PRO",
    "is_disabled": false,
    "is_forbidden": false,
    "quota_remaining": 85.5,
    "last_used": "2026-01-07T10:30:00Z"
  }
]
```

#### `POST /api/accounts`
Add a new account.

**Request:**
```json
{
  "refresh_token": "1//0abc...",
  "email": "user@gmail.com"
}
```

**Response:**
```json
{
  "success": true,
  "id": "new-account-id"
}
```

#### `DELETE /api/accounts/{id}`
Remove an account by ID.

**Response:**
```json
{
  "success": true
}
```

#### `POST /api/accounts/reload`
Hot-reload accounts from disk without restart.

**Response:**
```json
{
  "success": true,
  "accounts_loaded": 6
}
```

### Configuration

#### `GET /api/config`
Get current proxy configuration.

**Response:**
```json
{
  "port": 8045,
  "allow_lan_access": true,
  "request_timeout": 120,
  "anthropic_mapping": { ... },
  "openai_mapping": { ... },
  "adaptive_rate_limit": {
    "enabled": true,
    "safety_margin": 0.85,
    "aimd_increase": 0.05,
    "aimd_decrease": 0.7
  }
}
```

#### `POST /api/config/reload`
Hot-reload configuration from `gui_config.json`.

**Hot-Reloadable Fields:**
- `request_timeout`
- `anthropic_mapping`, `openai_mapping`, `custom_mapping`
- `sampling`, `pool_warming`, `log_rotation`
- `upstream_proxy`, `zai`, `scheduling`
- `enable_logging`
- `adaptive_rate_limit`

**Non-Reloadable Fields (require restart):**
- `port`, `allow_lan_access`, `auth_mode`

**Response:**
```json
{
  "success": true,
  "reloaded_fields": ["request_timeout", "adaptive_rate_limit"],
  "skipped_fields": ["port"],
  "message": "Changes: request_timeout: 60 -> 120"
}
```

### Statistics

#### `GET /api/stats`
Get request statistics.

**Response:**
```json
{
  "total_requests": 15420,
  "successful_requests": 15100,
  "failed_requests": 320,
  "total_tokens_in": 2500000,
  "total_tokens_out": 1800000,
  "requests_per_minute": 12.5,
  "avg_latency_ms": 1250
}
```

### Prometheus Metrics

#### `GET /metrics`
Prometheus-compatible metrics endpoint.

**Key Metrics:**
| Metric | Type | Description |
|--------|------|-------------|
| `antigravity_requests_total{status}` | Counter | Total requests by status |
| `antigravity_request_duration_seconds` | Histogram | Request latency |
| `antigravity_accounts_total` | Gauge | Total registered accounts |
| `antigravity_accounts_available` | Gauge | Available (non-rate-limited) accounts |
| `antigravity_circuit_breaker_state{account,state}` | Gauge | Circuit breaker states |
| `antigravity_adaptive_probes_total{strategy}` | Counter | Adaptive probing activity |
| `antigravity_aimd_rewards_total` | Counter | AIMD limit expansions |
| `antigravity_aimd_penalties_total` | Counter | AIMD limit contractions |
| `antigravity_log_files_total` | Gauge | Log files count |
| `antigravity_log_disk_bytes` | Gauge | Log disk usage |

## Rate Limiting

The Admin API is rate-limited to **60 requests per minute** per IP address.

Exceeding the limit returns:
```json
{
  "error": "Too Many Requests",
  "retry_after": 45
}
```

## Error Responses

All errors follow this format:
```json
{
  "error": "Error description",
  "code": "AG-001"
}
```

**Error Codes:**
| Code | Description |
|------|-------------|
| AG-001 | Authentication failed |
| AG-002 | Account not found |
| AG-003 | Invalid request body |
| AG-004 | Rate limit exceeded |
| AG-005 | Internal server error |
| AG-006 | Upstream timeout |
| AG-007 | All accounts exhausted |
| AG-008 | Circuit breaker open |

## Examples

### Check server health
```bash
curl https://antigravity.quantumind.ru/api/health
```

### List accounts
```bash
curl -H "Authorization: Bearer $API_KEY" \
  https://antigravity.quantumind.ru/api/accounts
```

### Hot-reload config
```bash
curl -X POST -H "Authorization: Bearer $API_KEY" \
  https://antigravity.quantumind.ru/api/config/reload
```

### Get Prometheus metrics
```bash
curl -H "Authorization: Bearer $API_KEY" \
  https://antigravity.quantumind.ru/metrics
```
