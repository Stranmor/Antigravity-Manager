# Alerting Configuration

The Antigravity Manager includes an automated health monitoring and alerting system that can send notifications to Telegram, Slack, or custom webhooks when system health degrades.

## Configuration

Add the following to your `gui_config.json` or set via environment variables:

```json
{
  "proxy": {
    "alerting": {
      "enabled": true,
      "check_interval_secs": 60,
      "cooldown_secs": 300,
      "telegram_bot_token": "YOUR_BOT_TOKEN",
      "telegram_chat_id": "YOUR_CHAT_ID",
      "slack_webhook_url": "https://hooks.slack.com/services/YOUR/WEBHOOK/URL",
      "custom_webhook_url": "https://your-monitoring-service.com/webhook"
    }
  }
}
```

### Configuration Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `enabled` | boolean | `false` | Enable/disable automated alerting |
| `check_interval_secs` | number | `60` | Health check interval in seconds |
| `cooldown_secs` | number | `300` | Minimum time between duplicate alerts (5 minutes) |
| `telegram_bot_token` | string | `null` | Telegram Bot API token (optional) |
| `telegram_chat_id` | string | `null` | Telegram chat/channel ID (optional) |
| `slack_webhook_url` | string | `null` | Slack incoming webhook URL (optional) |
| `custom_webhook_url` | string | `null` | Custom JSON webhook URL (optional) |

## Alert Rules

The system monitors the `/api/health/detailed` endpoint and triggers alerts based on the following conditions:

| Rule | Severity | Condition | Alert ID |
|------|----------|-----------|----------|
| System Unhealthy | 🚨 Critical | Overall status is "unhealthy" | `system_unhealthy` |
| System Degraded | ⚠️ Warning | Overall status is "degraded" | `system_degraded` |
| Low Available Accounts | ⚠️ Warning | Available accounts < 2 | `accounts_low` |
| High Circuit Breaker Failures | ⚠️ Warning | Open circuits > 50% of total accounts | `circuit_breaker_high` |
| All Accounts Unavailable | 🚨 Critical | No available accounts (service down) | `all_accounts_unavailable` |

## Telegram Setup

### 1. Create a Bot

1. Message [@BotFather](https://t.me/botfather) on Telegram
2. Send `/newbot` and follow the prompts
3. Copy the bot token (format: `123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11`)

### 2. Get Your Chat ID

**Option A: Personal Chat**
1. Message your bot
2. Visit: `https://api.telegram.org/bot<YOUR_BOT_TOKEN>/getUpdates`
3. Find `"chat":{"id":123456789}` in the JSON response

**Option B: Channel/Group**
1. Add the bot to your channel/group
2. Visit: `https://api.telegram.org/bot<YOUR_BOT_TOKEN>/getUpdates`
3. Find `"chat":{"id":-1001234567890}` (note the negative number for channels)

### 3. Test

```bash
curl -X POST "https://api.telegram.org/bot<YOUR_BOT_TOKEN>/sendMessage" \
  -H "Content-Type: application/json" \
  -d '{"chat_id":"<YOUR_CHAT_ID>","text":"Test message"}'
```

## Slack Setup

### 1. Create an Incoming Webhook

1. Go to [Slack App Directory](https://api.slack.com/messaging/webhooks)
2. Click "Create New App" → "From scratch"
3. Enable "Incoming Webhooks"
4. Click "Add New Webhook to Workspace"
5. Select channel and authorize
6. Copy the webhook URL (format: `https://hooks.slack.com/services/T00/B00/XXX`)

### 2. Test

```bash
curl -X POST "<YOUR_WEBHOOK_URL>" \
  -H "Content-Type: application/json" \
  -d '{"text":"Test message"}'
```

## Custom Webhook

The system can send alerts to any HTTP endpoint that accepts JSON POST requests.

### Request Format

```json
{
  "id": "accounts_low",
  "title": "⚠️ Low Available Accounts",
  "description": "Only 1 out of 6 accounts are available. Service may become unavailable soon.",
  "severity": "warning",
  "timestamp": 1704655200,
  "metadata": {
    "accounts_total": "6",
    "accounts_available": "1"
  }
}
```

### Severity Levels

- `info` - Informational alerts
- `warning` - Warning conditions that may require attention
- `critical` - Critical failures requiring immediate action

## Alert Deduplication

To prevent notification floods, the system implements smart deduplication:

- Each unique `alert_id` has a cooldown period (default: 300 seconds)
- Alerts within the cooldown period are automatically suppressed
- The system tracks fire count for each alert for analytics

Example: If "Low Available Accounts" alert fires at 10:00, it will NOT fire again until 10:05 even if the condition persists.

## Hot Reload

Alert configuration can be updated without restarting the server:

```bash
# Edit gui_config.json
nano ~/.antigravity/gui_config.json

# Reload configuration
curl -X POST http://localhost:9101/api/config/reload \
  -H "Authorization: Bearer YOUR_API_KEY"
```

## Monitoring Logs

View alerting activity in server logs:

```bash
# Follow live logs
ssh vps-production "journalctl -u antigravity-server -f | grep -i alert"

# Check last hour
ssh vps-production "journalctl -u antigravity-server --since '1 hour ago' | grep 'Sending alert'"
```

## Example Alert Messages

### Telegram

```
🚨 ALL ACCOUNTS UNAVAILABLE

Service is down. All accounts are rate-limited or unavailable.

Time: 2026-01-07 19:30:00 UTC
```

### Slack

![Slack Alert Example](https://via.placeholder.com/400x100/ff0000/FFFFFF?text=Critical+Alert)

Color-coded attachments:
- 🟢 Green: Info
- 🟠 Orange: Warning
- 🔴 Red: Critical

## Troubleshooting

### Alerts Not Sending

1. **Check configuration:**
   ```bash
   curl http://localhost:9101/api/config | jq '.alerting'
   ```

2. **Verify health endpoint:**
   ```bash
   curl http://localhost:9101/api/health/detailed | jq .
   ```

3. **Check logs for errors:**
   ```bash
   journalctl -u antigravity-server | grep -E "(alert|Alert)" | tail -20
   ```

### Webhook Returns Error

- **Telegram:** Ensure bot token and chat ID are correct
- **Slack:** Verify webhook URL is not expired
- **Custom:** Check endpoint accepts `application/json` POST

### Too Many Alerts

Increase `cooldown_secs`:
```json
{
  "alerting": {
    "cooldown_secs": 600
  }
}
```

## Production Recommendations

1. **Enable for VPS deployments:** Set `ANTIGRAVITY_ALERTING_ENABLED=true`
2. **Use dedicated alert channel:** Create #antigravity-alerts Slack channel
3. **Monitor alert frequency:** If getting flooded, adjust thresholds or increase cooldown
4. **Test before production:** Send test alerts to verify configuration

## Future Enhancements

Planned features for future releases:

- Alert history API endpoint
- Prometheus metrics integration
- Email notification support
- Alert scheduling (suppress during maintenance windows)
- Custom alert rule definitions via config
