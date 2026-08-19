# Cloudflare 侧配置

RelayDrop 经 Cloudflare 暴露中继的核心，是让 WebSocket 流量走 Cloudflare 边缘。
以下以 `relay.example.com` 为例。

## 步骤

### 1. 添加站点并解析

- 在 Cloudflare 添加 `example.com`。
- 添加 DNS 记录：`relay` → VPS 公网 IP（A 记录）。
- **代理状态设为 Proxied（橙色云）**。这是关键 —— 仅 DNS-only（灰色云）不会经 Cloudflare 边缘，
  客户端会直连 VPS IP。

### 2. 获取证书（供 nginx 终结 TLS）

两种方式二选一：

**A. Cloudflare Origin Certificate（推荐）**
- Cloudflare 控制台 → SSL/TLS → Origin Server → Create Certificate。
- 下载证书与私钥，放到 VPS（如 `/etc/nginx/certs/relay.example.com.pem` + `.key`）。
- 此证书仅对 `*.example.com` 有效，且只会被 Cloudflare 信任（适合 Authenticated Origin Pulls）。

**B. Let's Encrypt**
- 用 `certbot` 在 VPS 上签发（需 nginx 临时监听 80 做 HTTP-01 验证）。

### 3. SSL/TLS 模式

- 设为 **Full** 或 **Full (strict)**。
  - 用 Origin Certificate 时可用 Full (strict)。
  - 用自签/Let's Encrypt 时也建议 Full (strict)。
- 不要选 *Flexible*：它只在客户端↔Cloudflare 加密，Cloudflare↔VPS 是明文，且对 WebSocket 易出问题。

### 4. 确认 WebSocket 可用

Cloudflare 免费版原生代理 HTTPS/WebSocket（端口 443），无需特殊开关。
注意事项：

- **空闲超时**：Cloudflare 对 WebSocket 约有 **100 秒** 无数据的空闲超时。RelayDrop 的 `WsFramed`
  每 30 秒发送 WebSocket Ping 保活；在配对等待阶段也能维持连接。
- **大文件**：持续的数据流不会触发空闲超时；分块（64KB）持续发送即可。

### 5.（可选）Authenticated Origin Pulls

启用后，只有持有你 Origin Certificate 的 VPS 能被 Cloudflare 回源，进一步防伪造。
在 nginx 侧配置 `ssl_client_certificate` 指向 Cloudflare 的 Origin CA，并 `ssl_verify_client on;`。

## 不推荐的方案

- **直接把 VPS IP 直连当 `--relay`（`tcp://`/`ws://` 到 VPS IP）**：仅在 VPS 本身可达时有用；若需经 Cloudflare 边缘，则使用 `wss://`。
- **Spectrum**：Cloudflare Spectrum 可代理裸 TCP，但免费版不开放；本方案用 WebSocket 走 Cloudflare 免费版更通用。

## 验证

在本地（能访问 Cloudflare 的网络）执行：

```bash
relaydrop receive --relay wss://relay.example.com/relay --code MYCODE --password SECRET --out ./dl
# 另一端用相同 code/password 发送
```

中继日志应出现 `waiting in room` → `paired, piping`。
