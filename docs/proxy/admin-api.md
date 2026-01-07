# Admin API Reference

> Complete REST API documentation for the Antigravity Server headless deployment.

## Overview

The Admin API provides programmatic access to manage accounts, configuration, and monitor the proxy server. It runs on a separate port (default: `9101`) from the main proxy API (default: `8045`).

**Base URL:** `http://localhost:9101`

## Authentication

All endpoints except health checks require Bearer token authentication.

```bash
curl -H "Authorization: Bearer YOUR_API_KEY" http://localhost:9101/api/accounts
```

Set the API key via:
- Environment variable: `ANTIGRAVITY_API_KEY=your-secret-key`
- Config file: `gui_config.json` → `proxy.api_key`

### Public Endpoints (No Auth Required)
- `GET /api/health`
- `GET /api/health/summary`
- `GET /api/health/detailed`
- `GET /healthz`
- `GET /metrics`

---

## Health Endpoints

### GET /api/health

Basic health check with server status.

**Response:**
```json
{
  "status": "ok",
  "version": "3.3.15",
  "uptime_seconds": 3600,
  "accounts_total": 5,
  "accounts_available": 4,
  "proxy_running": true,
  "proxy_port": 8045
}
```

| Field | Type | Description |
|-------|------|-------------|
| `status` | string | `ok`, `degraded`, or `unhealthy` |
| `version` | string | Server version |
| `uptime_seconds` | integer | Seconds since server start |
| `accounts_total` | integer | Total registered accounts |
| `accounts_available` | integer | Accounts not rate-limited or disabled |
| `proxy_running` | boolean | Whether proxy server is active |
| `proxy_port` | integer | Proxy API port |

**Status Logic:**
- `ok`: accounts_available >= 50% of accounts_total
- `degraded`: accounts_available > 0 but < 50%
- `unhealthy`: accounts_total = 0 or accounts_available = 0

---

### GET /api/health/detailed

Comprehensive health check with component-level status.

**Response:**
```json
{
  "status": "healthy",
  "version": "3.3.15",
  "uptime_seconds": 3600,
  "components": {
    "database": {
      "status": "healthy",
      "latency_ms": 2
    },
    "token_manager": {
      "status": "healthy",
      "accounts_total": 5,
      "accounts_available": 4
    },
    "circuit_breaker": {
      "status": "healthy",
      "closed": 5,
      "open": 0,
      "half_open": 0
    },
    "proxy_server": {
      "status": "healthy"
    },
    "log_rotation": {
      "status": "healthy",
      "size_bytes": 1048576
    }
  },
  "checks": {
    "disk_space_ok": true,
    "memory_ok": true,
    "cpu_ok": true
  },
  "timestamp": "2026-01-07T12:00:00Z"
}
```

**Component Status Values:**
- `healthy`: Component operating normally
- `degraded`: Component functional but with issues
- `unhealthy`: Component not functioning

---

### GET /api/health/summary

Aggregated health summary for all accounts.

**Response:**
```json
{
  "total": 5,
  "healthy": 4,
  "degraded": 1,
  "unhealthy": 0,
  "accounts": [
    {
      "id": "abc123",
      "email": "user@example.com",
      "status": "healthy",
      "last_success": "2026-01-07T11:55:00Z",
      "error_rate": 0.02
    }
  ]
}
```

---

### GET /healthz

Kubernetes-style liveness probe (proxy port).

**Response:**
```json
{
  "status": "ok"
}
```

---

## Account Endpoints

### GET /api/accounts

List all registered accounts.

**Response:**
```json
[
  {
    "id": "abc123",
    "email": "user@example.com",
    "name": "Work Account",
    "disabled": false,
    "proxy_disabled": false,
    "created_at": 1704628800,
    "last_used": 1704715200
  }
]
```

| Field | Type | Description |
|-------|------|-------------|
| `id` | string | Unique account identifier |
| `email` | string | Google account email |
| `name` | string? | Optional display name |
| `disabled` | boolean | Account disabled due to OAuth error |
| `proxy_disabled` | boolean | Manually disabled from proxy pool |
| `created_at` | integer | Unix timestamp of creation |
| `last_used` | integer | Unix timestamp of last request |

---

### POST /api/accounts

Add a new account using a Google OAuth refresh token.

**Request:**
```json
{
  "refresh_token": "1//0abc...",
  "email": "user@example.com",
  "name": "My Account"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `refresh_token` | string | ✓ | Google OAuth refresh token |
| `email` | string | | Optional email (auto-detected) |
| `name` | string | | Optional display name |

**Response (201 Created):**
```json
{
  "id": "abc123",
  "email": "user@example.com",
  "name": "My Account"
}
```

**Errors:**
- `400 Bad Request`: Invalid refresh token
- `401 Unauthorized`: Token validation failed
- `409 Conflict`: Account already exists

---

### DELETE /api/accounts/{id}

Remove an account from the server.

**Response (204 No Content):** Success, no body.

**Errors:**
- `404 Not Found`: Account does not exist

---

### POST /api/accounts/reload

Reload all accounts from disk. Useful after manual file edits.

**Response:**
```json
{
  "accounts_loaded": 5
}
```

---

### GET /api/accounts/health

Get health status for all accounts.

**Response:**
```json
[
  {
    "id": "abc123",
    "email": "user@example.com",
    "status": "healthy",
    "circuit_state": "closed",
    "last_success": "2026-01-07T11:55:00Z",
    "last_error": null,
    "error_count_24h": 2,
    "success_count_24h": 150
  }
]
```

---

### GET /api/accounts/{id}/health

Get health status for a specific account.

**Response:** Same structure as single item from `/api/accounts/health`.

---

### POST /api/accounts/{id}/enable

Force re-enable a disabled account.

**Response:**
```json
{
  "success": true,
  "message": "Account abc123 re-enabled"
}
```

---

## Configuration Endpoints

### GET /api/config

Get current proxy configuration.

**Response:**
```json
{
  "port": 8045,
  "allow_lan_access": true,
  "auth_mode": "all_except_health",
  "request_timeout": 120,
  "anthropic_mapping": {...},
  "openai_mapping": {...},
  "scheduling": {
    "mode": "balance"
  },
  "log_rotation": {
    "enabled": true,
    "strategy": "daily",
    "max_files": 7
  }
}
```

---

### PUT /api/config

Update configuration (partial update supported).

**Request:**
```json
{
  "request_timeout": 180,
  "scheduling": {
    "mode": "performance"
  }
}
```

**Response:** Updated configuration object.

**Note:** Some fields require restart:
- `port`
- `allow_lan_access`
- `auth_mode`

---

### POST /api/config/reload

Hot-reload configuration from `gui_config.json` file on disk.

**Response:**
```json
{
  "success": true,
  "reloaded_fields": [
    "request_timeout",
    "anthropic_mapping",
    "openai_mapping",
    "custom_mapping",
    "sampling",
    "pool_warming",
    "log_rotation",
    "upstream_proxy",
    "zai",
    "scheduling",
    "enable_logging"
  ],
  "skipped_fields": [
    "port",
    "allow_lan_access",
    "auth_mode"
  ],
  "message": "Changes: request_timeout: 60 -> 120"
}
```

---

## Statistics Endpoints

### GET /api/stats

Get proxy request statistics.

**Response:**
```json
{
  "total_requests": 15000,
  "successful_requests": 14850,
  "failed_requests": 150,
  "active_connections": 3,
  "requests_per_minute": 25.5,
  "avg_latency_ms": 1250,
  "p95_latency_ms": 3500,
  "p99_latency_ms": 5000
}
```

---

### GET /metrics

Prometheus metrics endpoint.

**Response:** Prometheus text format.

```
# HELP antigravity_requests_total Total requests processed
# TYPE antigravity_requests_total counter
antigravity_requests_total{status="success"} 14850
antigravity_requests_total{status="error"} 150

# HELP antigravity_request_duration_seconds Request duration histogram
# TYPE antigravity_request_duration_seconds histogram
antigravity_request_duration_seconds_bucket{le="0.1"} 100
antigravity_request_duration_seconds_bucket{le="0.5"} 5000
...
```

**Key Metrics:**
| Metric | Type | Description |
|--------|------|-------------|
| `antigravity_requests_total` | Counter | Total requests by status |
| `antigravity_request_duration_seconds` | Histogram | Request latency |
| `antigravity_accounts_total` | Gauge | Total accounts |
| `antigravity_accounts_available` | Gauge | Available accounts |
| `antigravity_uptime_seconds` | Gauge | Server uptime |
| `antigravity_circuit_breaker_state` | Gauge | Circuit breaker states |
| `antigravity_adaptive_probes_total` | Counter | AIMD probe requests |
| `antigravity_aimd_rewards_total` | Counter | AIMD limit increases |
| `antigravity_aimd_penalties_total` | Counter | AIMD limit decreases |
| `antigravity_log_files_total` | Gauge | Log files count |
| `antigravity_log_disk_bytes` | Gauge | Log disk usage |

---

## Database Backup Endpoints

### POST /api/db/backup

Create a database backup.

**Response:**
```json
{
  "success": true,
  "backup_path": "/var/lib/antigravity/backups/proxy_logs_2026-01-07T12-00-00.db",
  "size_bytes": 1048576,
  "timestamp": "2026-01-07T12:00:00Z"
}
```

---

### GET /api/db/backups

List available backups.

**Response:**
```json
{
  "backups": [
    {
      "path": "/var/lib/antigravity/backups/proxy_logs_2026-01-07T12-00-00.db",
      "size_bytes": 1048576,
      "created_at": "2026-01-07T12:00:00Z",
      "age_hours": 2
    }
  ]
}
```

---

### POST /api/db/restore

Restore from a backup file.

**Request:**
```json
{
  "backup_path": "/var/lib/antigravity/backups/proxy_logs_2026-01-07T12-00-00.db"
}
```

**Response:**
```json
{
  "success": true,
  "restored_from": "/var/lib/antigravity/backups/proxy_logs_2026-01-07T12-00-00.db",
  "message": "Database restored successfully. Server restart recommended."
}
```

---

## Error Responses

All error responses follow this format:

```json
{
  "error": "Error message description",
  "code": "ERROR_CODE"
}
```

### HTTP Status Codes

| Code | Description |
|------|-------------|
| 200 | Success |
| 201 | Created |
| 204 | No Content (success, no body) |
| 400 | Bad Request |
| 401 | Unauthorized (missing/invalid API key) |
| 404 | Not Found |
| 409 | Conflict |
| 429 | Too Many Requests (rate limited) |
| 500 | Internal Server Error |

### Rate Limiting

The Admin API is rate-limited to **60 requests per minute** per client IP.

When rate-limited, responses include:
```
HTTP/1.1 429 Too Many Requests
Retry-After: 30
X-RateLimit-Limit: 60
X-RateLimit-Remaining: 0
X-RateLimit-Reset: 1704715200
```

---

## Usage Examples

### cURL

```bash
# Check health
curl http://localhost:9101/api/health

# List accounts (with auth)
curl -H "Authorization: Bearer $API_KEY" \
     http://localhost:9101/api/accounts

# Add account
curl -X POST \
     -H "Authorization: Bearer $API_KEY" \
     -H "Content-Type: application/json" \
     -d '{"refresh_token": "1//0abc..."}' \
     http://localhost:9101/api/accounts

# Hot-reload config
curl -X POST \
     -H "Authorization: Bearer $API_KEY" \
     http://localhost:9101/api/config/reload

# Create backup
curl -X POST \
     -H "Authorization: Bearer $API_KEY" \
     http://localhost:9101/api/db/backup
```

### Python

```python
import requests

API_KEY = "your-api-key"
BASE_URL = "http://localhost:9101"

headers = {"Authorization": f"Bearer {API_KEY}"}

# List accounts
accounts = requests.get(f"{BASE_URL}/api/accounts", headers=headers).json()

# Add account
response = requests.post(
    f"{BASE_URL}/api/accounts",
    headers=headers,
    json={"refresh_token": "1//0abc..."}
)

# Get stats
stats = requests.get(f"{BASE_URL}/api/stats", headers=headers).json()
```

### JavaScript/TypeScript

```typescript
const API_KEY = 'your-api-key';
const BASE_URL = 'http://localhost:9101';

const headers = { 'Authorization': `Bearer ${API_KEY}` };

// List accounts
const accounts = await fetch(`${BASE_URL}/api/accounts`, { headers })
  .then(r => r.json());

// Add account
const newAccount = await fetch(`${BASE_URL}/api/accounts`, {
  method: 'POST',
  headers: { ...headers, 'Content-Type': 'application/json' },
  body: JSON.stringify({ refresh_token: '1//0abc...' })
}).then(r => r.json());
```

---

## Changelog

- **v3.3.15**: Added adaptive rate limiting metrics, AIMD probes
- **v3.3.14**: Added `/api/db/backup`, `/api/db/restore` endpoints
- **v3.3.13**: Added `/api/config/reload` for hot-reload
- **v3.3.12**: Added `/api/health/detailed` with component status
- **v3.3.11**: Added `/api/accounts/{id}/enable` for force re-enable
