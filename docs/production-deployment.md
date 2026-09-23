# NOVA DB — Production Deployment Guide

This guide details recommended infrastructure configurations, system tuning, and operational practices for deploying NOVA DB in high-availability environments.

---

## 1. Operating System & Kernel Tuning

To achieve sub-millisecond tail latencies under high connection loads, apply the following Linux kernel settings:

```ini
# /etc/sysctl.d/99-nova.conf

# Increase file descriptor limit
fs.file-max = 2097152

# Socket buffer sizes for high-bandwidth NVP streams
net.core.rmem_max = 16777216
net.core.wmem_max = 16777216
net.ipv4.tcp_rmem = 4096 87380 16777216
net.ipv4.tcp_wmem = 4096 65536 16777216

# Max pending network connections
net.core.somaxconn = 4096
net.ipv4.tcp_max_syn_backlog = 8192
```

---

## 2. Storage System Configuration

1. **NVMe / SSD Storage**: Ensure Write-Ahead Log segments (`.wal`) are mounted on solid-state drives with write barriers enabled.
2. **File System**: Prefer `ext4` or `xfs` formatted with `noatime,nodiratime` mount options.
3. **Fsync Durability**:
   - For mission-critical banking/auditing: Use `--sync always` (default).
   - For high-throughput analytics or metrics: Use `--sync none` to rely on OS page cache batching.

---

## 3. Containerized Deployment via Docker

```bash
# Build and run using Docker Compose
docker compose up -d

# Verify server health and connectivity
docker compose exec nova-db nova status --addr 127.0.0.1:7400
```

---

## 4. Monitoring & Telemetry

NOVA DB exposes Prometheus-compatible metrics via the REST gateway:

```bash
# Fetch machine-readable cluster metrics
curl -s http://127.0.0.1:7401/api/status | jq .metrics
```
