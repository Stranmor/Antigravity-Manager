# Operations Guide - Antigravity Manager

## VPS Health Monitoring

### Overview
Antigravity Manager includes automated health monitoring with auto-restart capabilities to ensure 24/7 availability.

### Components

#### 1. Health Check Script
**Location:** `scripts/monitor-vps.sh`

**Features:**
- Polls `https://antigravity.quantumind.ru/healthz` every 5 minutes
- Tracks consecutive failures (max 2 = 10 minutes)
- Automatically restarts service via SSH on persistent failures
- Logs all operations to `/tmp/antigravity-monitor.log`

**Manual Testing:**
```bash
# Test health check
./scripts/monitor-vps.sh

# View failure counter
cat /tmp/antigravity-health-failures

# View logs
tail -f /tmp/antigravity-monitor.log
```

#### 2. Systemd Timer (Alternative to Cron)
**Location:** `deploy/systemd/antigravity-monitor.{service,timer}`

**Installation:**
```bash
# Copy unit files to systemd user directory
mkdir -p ~/.config/systemd/user
cp deploy/systemd/antigravity-monitor.service ~/.config/systemd/user/
cp deploy/systemd/antigravity-monitor.timer ~/.config/systemd/user/

# Enable and start timer
systemctl --user daemon-reload
systemctl --user enable --now antigravity-monitor.timer

# Check timer status
systemctl --user status antigravity-monitor.timer
systemctl --user list-timers
```

**Verify Timer:**
```bash
# View timer status
systemctl --user status antigravity-monitor.timer

# View last run
journalctl --user -u antigravity-monitor.service -n 20

# Manually trigger check (for testing)
systemctl --user start antigravity-monitor.service
```

#### 3. Cron Alternative (If Available)
If your system uses traditional cron:

```bash
# Add to crontab
crontab -e

# Add this line:
*/5 * * * * /home/stranmor/Documents/project/_mycelium/Antigravity-Manager/scripts/monitor-vps.sh >> /tmp/antigravity-monitor.log 2>&1

# Verify cron entry
crontab -l | grep antigravity
```

### Configuration

Edit `scripts/monitor-vps.sh` to customize:

```bash
HEALTH_URL="https://antigravity.quantumind.ru/healthz"  # Health endpoint
MAX_FAILURES=2          # Restart after N consecutive failures
TIMEOUT=10              # Health check timeout (seconds)
VPS_HOST="vps-production"  # SSH hostname
```

### Monitoring & Troubleshooting

#### View Live Logs
```bash
tail -f /tmp/antigravity-monitor.log
```

#### Log Format
- `✅` - Health check passed
- `❌` - Health check failed
- `🔄` - Service restart triggered
- `⚠️` - Service degraded or warning state

#### Common Issues

**Issue:** Script fails with "Connection refused"
- **Cause:** VPS is down or SSH key not configured
- **Fix:** Verify SSH access with `ssh vps-production`

**Issue:** Script reports "401 Unauthorized"
- **Cause:** Using wrong health endpoint (e.g., `/api/health/detailed`)
- **Fix:** Use public `/healthz` endpoint instead

**Issue:** Timer not running
- **Cause:** Timer not enabled or systemd user service not started
- **Fix:** 
  ```bash
  systemctl --user enable --now antigravity-monitor.timer
  loginctl enable-linger $USER  # Allow user services to run at boot
  ```

**Issue:** Too many false positives
- **Cause:** Network instability or VPS slow response
- **Fix:** Increase `MAX_FAILURES` to 3-4 or `TIMEOUT` to 15-20 seconds

### Alert Integration

The monitoring script can trigger AlertManager webhooks on restart. To enable:

1. Edit VPS config `/var/lib/antigravity/gui_config.json`:
```json
{
  "proxy": {
    "alerting": {
      "enabled": true,
      "telegram_bot_token": "YOUR_BOT_TOKEN",
      "telegram_chat_id": "432567587"
    }
  }
}
```

2. Restart VPS service:
```bash
ssh vps-production "sudo systemctl restart antigravity-server"
```

### Metrics & Analytics

**Health Check Success Rate:**
```bash
# Count total checks
grep -c "Health check" /tmp/antigravity-monitor.log

# Count failures
grep -c "❌" /tmp/antigravity-monitor.log

# Count restarts
grep -c "🔄" /tmp/antigravity-monitor.log
```

**Average Recovery Time:**
- Monitor logs for time between "🔄 Restarting" and "✅ Service recovered"

### Best Practices

1. **Enable Persistent Logging:**
   ```bash
   # Rotate logs weekly
   sudo logrotate -f /etc/logrotate.d/antigravity-monitor
   ```

2. **Monitor SSH Key Expiry:**
   - Ensure `~/.ssh/id_rsa` for vps-production doesn't expire
   - Use `ssh-add -l` to verify agent keys

3. **Test Restart Manually:**
   ```bash
   # Simulate failure by stopping service
   ssh vps-production "sudo systemctl stop antigravity-server"
   
   # Wait 10 minutes, verify auto-restart
   sleep 600
   ssh vps-production "systemctl status antigravity-server"
   ```

4. **Set Up Alerting:**
   - Configure Telegram/Slack webhooks on VPS
   - Monitor `/tmp/antigravity-monitor.log` for restart patterns

### Security Considerations

- Script requires **passwordless SSH** to vps-production
- Ensure SSH key is protected with passphrase + ssh-agent
- Monitor sudo access logs on VPS: `journalctl -u sudo`

### Uninstallation

**Systemd Timer:**
```bash
systemctl --user stop antigravity-monitor.timer
systemctl --user disable antigravity-monitor.timer
rm ~/.config/systemd/user/antigravity-monitor.{service,timer}
systemctl --user daemon-reload
```

**Cron:**
```bash
crontab -e
# Remove antigravity line
```

**Cleanup:**
```bash
rm /tmp/antigravity-health-failures
rm /tmp/antigravity-monitor.log
```

---

## Metrics Analysis Dashboard

### Fetch Production Metrics
```bash
# Query all antigravity metrics from VPS
ssh vps-production "curl -s http://localhost:9101/metrics" | grep "^antigravity_"

# Parse specific metrics
ssh vps-production "curl -s http://localhost:9101/metrics" | grep "antigravity_request_duration"
```

### Key Metrics to Monitor

#### 1. Request Latency
```bash
# P95 latency
ssh vps-production "curl -s http://localhost:9101/metrics" | grep 'quantile="0.95"'

# P99 latency
ssh vps-production "curl -s http://localhost:9101/metrics" | grep 'quantile="0.99"'
```

**Thresholds:**
- P95 < 5s: Healthy
- P95 5-10s: Degraded
- P95 > 10s: Critical

#### 2. Circuit Breaker State
```bash
ssh vps-production "curl -s http://localhost:9101/metrics" | grep "antigravity_circuit_breaker_state"
```

**Values:**
- 0 = Closed (healthy)
- 1 = Open (failing)
- 2 = Half-Open (testing recovery)

#### 3. Rate Limit Events
```bash
ssh vps-production "curl -s http://localhost:9101/metrics" | grep "antigravity_rate_limit"
```

#### 4. Account Availability
```bash
ssh vps-production "curl -s http://localhost:9101/metrics" | grep "antigravity_accounts_available"
```

### Automated Analysis Script
**Location:** `scripts/analyze-metrics.sh` (to be created in next phase)

**Features:**
- Fetch and parse Prometheus metrics
- Calculate percentiles
- Detect anomalies (latency spikes, high error rates)
- Generate recommendations (rebalance accounts, adjust quotas)

---

## VPS Management Commands

### Service Control
```bash
# Status
ssh vps-production "systemctl status antigravity-server"

# Restart
ssh vps-production "sudo systemctl restart antigravity-server"

# View logs
ssh vps-production "journalctl -u antigravity-server -f"

# View last 100 lines
ssh vps-production "journalctl -u antigravity-server -n 100"
```

### Health Checks
```bash
# Basic health
curl https://antigravity.quantumind.ru/healthz

# Detailed health (requires auth)
curl -H "Authorization: Bearer sk-ag-eXtP3wWkF5WIXWbNfZnnmDmMcvGYSsckHW1OcFGrvk" \
  https://antigravity.quantumind.ru/api/health/detailed | jq

# Metrics
curl https://antigravity.quantumind.ru/metrics
```

### Database Operations
```bash
# Backup database
ssh vps-production "sudo cp /var/lib/antigravity/proxy_logs.db /var/lib/antigravity/backup-$(date +%Y%m%d).db"

# Check database size
ssh vps-production "du -h /var/lib/antigravity/proxy_logs.db"

# Vacuum database (shrink)
ssh vps-production "sqlite3 /var/lib/antigravity/proxy_logs.db 'VACUUM;'"
```

### Log Management
```bash
# Check log disk usage
ssh vps-production "du -h /var/lib/antigravity/logs/"

# List rotated logs
ssh vps-production "ls -lh /var/lib/antigravity/logs/"

# View latest log
ssh vps-production "tail -100 /var/lib/antigravity/logs/server.log"
```

### Configuration Updates
```bash
# Edit config
ssh vps-production "sudo nano /var/lib/antigravity/gui_config.json"

# Hot-reload config (no restart)
curl -X POST -H "Authorization: Bearer sk-ag-eXtP3wWkF5WIXWbNfZnnmDmMcvGYSsckHW1OcFGrvk" \
  https://antigravity.quantumind.ru/api/config/reload

# Verify reload
ssh vps-production "journalctl -u antigravity-server -n 20"
```

---

## Emergency Procedures

### Service Down
1. Check health: `curl https://antigravity.quantumind.ru/healthz`
2. Check VPS access: `ssh vps-production`
3. Check service: `ssh vps-production "systemctl status antigravity-server"`
4. View logs: `ssh vps-production "journalctl -u antigravity-server -n 50"`
5. Restart: `ssh vps-production "sudo systemctl restart antigravity-server"`

### All Accounts Unavailable
1. Check metrics: `curl https://antigravity.quantumind.ru/metrics | grep accounts_available`
2. Review admin API: `curl -H "Authorization: Bearer ..." https://antigravity.quantumind.ru/api/accounts`
3. Check for rate limits: `ssh vps-production "journalctl -u antigravity-server | grep '429'"`
4. Reload accounts: `curl -X POST -H "Authorization: Bearer ..." https://antigravity.quantumind.ru/api/accounts/reload`

### High Latency (P95 > 10s)
1. Check circuit breaker: `curl https://antigravity.quantumind.ru/metrics | grep circuit_breaker`
2. Identify slow accounts: `ssh vps-production "journalctl -u antigravity-server | grep 'latency_ms'"`
3. Disable slow accounts via Admin API
4. Monitor recovery: watch latency metrics for 10 minutes

### Database Corruption
1. Stop service: `ssh vps-production "sudo systemctl stop antigravity-server"`
2. Backup corrupted DB: `ssh vps-production "sudo cp /var/lib/antigravity/proxy_logs.db /tmp/corrupted.db"`
3. Restore from backup: `ssh vps-production "sudo cp /var/lib/antigravity/backup-YYYYMMDD.db /var/lib/antigravity/proxy_logs.db"`
4. Start service: `ssh vps-production "sudo systemctl start antigravity-server"`

---

## Maintenance Checklist

### Daily
- [ ] Check `/tmp/antigravity-monitor.log` for restart events
- [ ] Verify systemd timer is running: `systemctl --user status antigravity-monitor.timer`

### Weekly
- [ ] Review metrics trends (latency, error rate, account usage)
- [ ] Check log disk usage on VPS
- [ ] Backup VPS database

### Monthly
- [ ] Update VPS binary: `./scripts/deploy-vps.sh`
- [ ] Review and rotate old logs
- [ ] Test disaster recovery (manual service stop + auto-restart)
- [ ] Update monitoring thresholds based on traffic patterns
