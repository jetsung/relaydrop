# 安装与部署中继

本文说明如何在 VPS 上部署 relaydrop 中继，并提供三种部署方式。

## 构建 / 获取二进制

在 VPS（或任意可编译的机器）上：

```bash
cargo build --release
# 产物: target/release/relaydrop
```

将 `relaydrop` 二进制放到 VPS（如 `/usr/local/bin/relaydrop`）。

## 中继的端口与证书策略

relaydrop 的中继 **只监听一个 TCP 端口，使用明文 WebSocket（或裸 TCP）**，本身不处理 TLS、不占用 443。
TLS 终结由前端组件负责：

- VPS 的 443/80 通常被既有 Web 服务（nginx）占用 —— 用 nginx 反向代理（推荐，见 [nginx.md](nginx.md)）。
- 不想动 nginx / 没有开放端口 —— 用 `cloudflared` tunnel（见下文）。
- 443 空闲且想让中继直接对外 —— 用 Cloudflare 源证书（见 [cloudflare.md](cloudflare.md)）。

## 方式一：nginx 反向代理（推荐）

中继仅监听本地：

```bash
relaydrop relay --listen 127.0.0.1:9090 --password <RELAY_PASSWORD>
```

nginx 在 `:443` 终结 TLS，并把 `wss://relay.example.com/relay` 代理到 `http://127.0.0.1:9090`。
完整 nginx 配置见 [nginx.md](nginx.md)。Cloudflare 侧把 `relay.example.com` 设为橙色云（Proxied）。

## 方式二：cloudflared tunnel

无需开放任何公网端口。完整配置（命名隧道 + systemd 守护）见 [cloudflared.md](cloudflared.md)。

```bash
relaydrop relay --listen 127.0.0.1:9090 --password <RELAY_PASSWORD>
cloudflared tunnel --url ws://127.0.0.1:9090
```

`cloudflared` 会分配一个 `*.trycloudflare.com` 域名（或你在 Cloudflare 配置的隧道主机名）。
客户端用 `wss://<tunnel-host>/relay` 连接。

## 方式三：443 空闲时由前端卸载证书（不推荐）

> **重要**：中继**只做明文监听**（裸 WebSocket/TCP），自身不持有证书、不终止 TLS、也不监听 443。
> 下面的「443」指的是由 nginx / cloudflared 在前端持有证书并终结 TLS，中继始终明文监听**本地**端口。

若 VPS 的 443 当前空闲（没有其他 Web 服务占用），可让 nginx 在该 443 上终结 TLS、把
`wss://relay.example.com/relay` 代理到本地明文中继 `http://127.0.0.1:9090`：

```bash
# 中继：仍然明文、监听本地
relaydrop relay --listen 127.0.0.1:9090 --password <RELAY_PASSWORD>
```

```nginx
# nginx 在 443 终结 TLS（证书用 Cloudflare Origin Certificate 或 Let's Encrypt）
server {
    listen 443 ssl;
    server_name relay.example.com;
    ssl_certificate     /etc/nginx/certs/relay.example.com.pem;
    ssl_certificate_key /etc/nginx/certs/relay.example.com.key;
    location /relay {
        proxy_pass http://127.0.0.1:9090;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 3600s;
    }
}
```

演示步骤与排错见 [nginx.md](nginx.md) 与 [cloudflare.md](cloudflare.md)。
绝大多数情况下直接用「方式一 nginx（推荐）」或「方式二 cloudflared」即可，无需手动摆弄 443。

## 守护进程（systemd 示例）

> **不要把口令写进命令行参数**：命令行会出现在 `/proc/*/cmdline` 与 shell 历史，任何同机用户都能读到。
> 推荐用 `EnvironmentFile` 存放口令（权限 `600`，属专用用户）。

先把口令写入受限文件：

```bash
install -d -m 700 -o relaydrop -g relaydrop /etc/relaydrop
echo 'RELAYDROP_PASSWORD=<RELAY_PASSWORD>' > /etc/relaydrop/password
chmod 600 /etc/relaydrop/password
chown relaydrop:relaydrop /etc/relaydrop/password
```

再写单元文件：

```ini
# /etc/systemd/system/relaydrop.service
[Unit]
Description=relaydrop relay
After=network.target

[Service]
Type=simple
User=relaydrop
EnvironmentFile=/etc/relaydrop/password
ExecStart=/usr/local/bin/relaydrop relay --listen 127.0.0.1:9090 --password ${RELAYDROP_PASSWORD}
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

```bash
systemctl daemon-reload
systemctl enable --now relaydrop
```

> 更高安全场景可用 systemd `LoadCredential=` 替代 `EnvironmentFile`，口令不会以环境变量形式暴露。

### 用环境变量注入参数

relaydrop 的每个参数也都支持环境变量（见 [usage.md](usage.md) 的「环境变量」小节），命名为 `RELAYDROP_<参数大写>`。
systemd 通过 `Environment=` / `EnvironmentFile=` 注入的变量会被中继进程继承，clap 直接读取，
因此上面的 `--password ${CROC_PASSWORD}` 也可以改为完全走环境变量：

```ini
[Service]
Type=simple
User=relaydrop
Environment=RELAYDROP_LISTEN=127.0.0.1:9090
Environment=RELAYDROP_PASSWORD=SECRET
# 也可改用 EnvironmentFile=/etc/relaydrop/env（内含 RELAYDROP_LISTEN / RELAYDROP_PASSWORD 等）
ExecStart=/usr/local/bin/relaydrop relay
Restart=on-failure
```

这样命令行里不再出现任何敏感值，且所有参数（`--listen`、`--ttl` 等）都能用同样方式配置。

## 中继参数

| 参数 | 默认 | 说明 |
| --- | --- | --- |
| `--listen` | `127.0.0.1:9090` | 监听地址（明文 ws/tcp 混用） |
| `--password` | 空 | 中继口令，客户端必须一致才能鉴权通过 |
| `--ttl` | `300` | 房间未被配对时的存活秒数，超时自动清理 |
