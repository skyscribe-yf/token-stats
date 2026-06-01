# ECS Deployment: Exposing Local Dashboard via Reverse Tunnel

This document describes how the token-stats dashboard, running on a local/WSL2 machine, is exposed to the internet through an ECS server using an SSH reverse tunnel and nginx reverse proxy.

---

## Architecture Overview

```
Internet (https://<DOMAIN>/token-stats/)
  │
  ▼
┌─────────────────────────────────────────┐
│  ECS Server (<ECS_IP>)                  │
│                                         │
│  Port 443 → sslh (SSH/SSL multiplexer) │
│    ├── SSH traffic → 127.0.0.1:2222     │
│    └── HTTPS traffic → 127.0.0.1:8443   │
│                                         │
│  nginx (port 8443, HTTPS)               │
│    └── /token-stats/ → 127.0.0.1:3002   │
│                                         │
│  Port 3002 ← SSH reverse tunnel         │
└─────────────┬───────────────────────────┘
              │ SSH tunnel (-R 3002:localhost:80)
              ▼
┌─────────────────────────────────────────┐
│  Local Machine                          │
│                                         │
│  nginx (port 80, server_name localhost) │
│    └── /token-stats/ → upstream backend │
│                                         │
│  Backend (port 3000 or 3001)            │
│    └── blue-green deployment            │
└─────────────────────────────────────────┘
```

**Key principle:** ECS only sees `localhost:3002` (the tunnel endpoint). All blue-green port switching (3000 ↔ 3001) is handled by the **local** nginx, so ECS config never changes during deployment.

---

## Components

### 1. sslh — SSL/SSH Multiplexer (ECS)

Shares port 443 between SSH and HTTPS, so both `ssh <ECS_IP> -p 443` and `https://<DOMAIN>/` work on the same port.

**Config:** `/etc/sslh.cfg`

```cfg
listen:
(
    { host: "0.0.0.0"; port: "443"; }
);

protocols:
(
    { name: "ssh";  host: "127.0.0.1"; port: "2222"; },
    { name: "ssl";  host: "127.0.0.1"; port: "8443"; }
);
```

### 2. ECS nginx — HTTPS Reverse Proxy (ECS)

Listens on port 8443 (behind sslh). Serves multiple projects via include files:

**Main config:** `/etc/nginx/conf.d/<MAIN_CONF>.conf`

```
server {
    listen 8443 ssl;
    server_name _;

    ssl_certificate     /etc/nginx/ssl/<CERT_FILE>.crt;
    ssl_certificate_key /etc/nginx/ssl/<CERT_FILE>.key;

    include /etc/nginx/<PROJECT_A>-locations.inc;
    include /etc/nginx/<PROJECT_B>-locations.inc;
    include /etc/nginx/token-stats-locations.inc;   # ← token-stats
    location / { return 404; }
}
```

**token-stats include:** `/etc/nginx/token-stats-locations.inc`

```nginx
location /token-stats/ {
    proxy_pass http://127.0.0.1:3002/token-stats/;
    proxy_http_version 1.1;
    # CRITICAL: Host must be "localhost" to match local nginx server_name.
    # Using $host (the public domain) causes local nginx to return 404.
    proxy_set_header Host localhost;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
    proxy_set_header Connection "";
    proxy_read_timeout 60s;
}
```

Source file in repo: `nginx/ecs-token-stats-location.conf`

### 3. SSH Reverse Tunnel (Local → ECS)

Established by `autossh` from the local machine, creating a persistent reverse tunnel:

**ECS:3002 → localhost:80**

The local machine pushes port 80 (local nginx) to ECS port 3002. ECS nginx then proxies to `127.0.0.1:3002`.

**systemd service:** `~/.config/systemd/user/ecs-tunnel.service`

```ini
[Unit]
Description=SSH Reverse Tunnel to ECS
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment="AUTOSSH_GATETIME=0"
Environment="AUTOSSH_POLL=30"
ExecStart=/usr/bin/autossh -M 0 -N -T \
  -o "ServerAliveInterval=60" \
  -o "ServerAliveCountMax=3" \
  -o "ExitOnForwardFailure=yes" \
  -R 3002:localhost:80 \
  ecs
Restart=always
RestartSec=10

[Install]
WantedBy=default.target
```

**SSH config** (`~/.ssh/config`):

```
Host ecs
    User <ECS_USER>
    Hostname <ECS_IP>
```

**Setup commands:**

```bash
# Enable and start the tunnel
systemctl --user enable ecs-tunnel.service
systemctl --user start ecs-tunnel.service

# Check status
systemctl --user status ecs-tunnel.service

# Verify tunnel is active on ECS
ssh ecs "ss -tlnp | grep 3002"
```

### 4. Local nginx — Blue-Green Proxy (Local)

Listens on port 80 with `server_name localhost`. Strips the `/token-stats/` prefix and forwards to the backend.

**Config:** `/etc/nginx/sites-available/token-stats` (deployed by `deploy.sh`)

```nginx
upstream token_stats_backend {
    server 127.0.0.1:3000;   # ← swapped between 3000/3001 by deploy.sh
    keepalive 32;
}

server {
    listen 80 default_server;
    listen [::]:80 default_server;
    server_name localhost;

    location /token-stats/ {
        proxy_pass http://token_stats_backend/;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Connection "";
        proxy_read_timeout 60s;
    }
}
```

Source file in repo: `nginx/token-stats.conf`

---

## Deployment Workflow

### Local blue-green deployment (no ECS changes)

`./deploy.sh` handles zero-downtime deploys entirely on the local machine:

1. Build new backend + frontend
2. Start new backend on the alternate port (3000 ↔ 3001)
3. Health check new instance
4. Update local nginx upstream to new port
5. Reload local nginx — **traffic switches instantly**
6. Drain and stop old instance

**ECS is never involved** — the tunnel (`ECS:3002 → local:80`) and ECS nginx remain untouched.

### Initial ECS setup (one-time)

1. **Copy the location include** to ECS:

   ```bash
   scp nginx/ecs-token-stats-location.conf ecs:/etc/nginx/token-stats-locations.inc
   ```

2. **Add the include** to the HTTPS server block in ECS nginx config:

   ```
   include /etc/nginx/token-stats-locations.inc;
   ```

3. **Test and reload** ECS nginx:

   ```bash
   ssh ecs "nginx -t && nginx -s reload"
   ```

4. **Set up the SSH tunnel** on the local machine (see section 3 above).

---

## Troubleshooting

### Dashboard returns 404 on public URL

1. **Check tunnel is alive:**

   ```bash
   ssh ecs "ss -tlnp | grep 3002"
   ssh ecs "curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:3002/token-stats/"
   ```

   If tunnel is down, restart: `systemctl --user restart ecs-tunnel.service`

2. **Check local backend:**

   ```bash
   curl -s -o /dev/null -w '%{http_code}' http://localhost/token-stats/
   ```

   If 404, the backend may be down — check: `systemctl status token-stats@3000`

3. **Check Host header** — the most common cause:

   ECS nginx must send `Host: localhost`, NOT `Host: $host`. Verify:

   ```bash
   ssh ecs "grep 'proxy_set_header Host' /etc/nginx/token-stats-locations.inc"
   ```

   Must show `proxy_set_header Host localhost;`. If it shows `$host`, local nginx won't match the `server_name localhost` block and returns 404.

### Tunnel keeps disconnecting

- `autossh` auto-reconnects (restarts the SSH session on failure).
- Check logs: `journalctl --user -u ecs-tunnel.service -f`
- Verify SSH key auth works: `ssh ecs "echo ok"` (should not prompt for password)

### ECS nginx not picking up changes

```bash
ssh ecs "nginx -t && nginx -s reload"
```

---

## Port Reference

| Port | Where | Purpose |
|------|-------|---------|
| 443 | ECS | Public HTTPS + SSH (sslh multiplexed) |
| 2222 | ECS | SSH target (from sslh SSH routing) |
| 3002 | ECS | SSH reverse tunnel endpoint → local:80 |
| 8443 | ECS | nginx HTTPS (behind sslh) |
| 80 | Local | nginx reverse proxy (blue-green switch) |
| 3000 | Local | Backend instance A (blue) |
| 3001 | Local | Backend instance B (green) |

---

## Security Notes

- **No backend port is exposed** — only ECS port 443 is public; all backend ports are localhost-only.
- **SSH key auth** — the tunnel uses key-based SSH auth, no passwords.
- **sslh** — multiplexes SSH and HTTPS on port 443, useful for networks that block port 22.
- **Host header isolation** — local nginx only matches `server_name localhost`, so direct requests with other Host headers are rejected (404), preventing unintended access.
