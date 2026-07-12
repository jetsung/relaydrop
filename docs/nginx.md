# nginx 反向代理配置

当 VPS 的 443/80 已被 nginx 占用时，用 nginx 在 `:443` 终结 TLS 并把 WebSocket 转发给本地中继。
这是推荐的部署方式：中继不持证书、不占 443，Cloudflare 橙色云指向 nginx。

## 前提

- 域名 `relay.example.com` 已解析到 VPS，并在 Cloudflare 设为 **Proxied（橙色云）**。
- 已获得该域名的证书（Cloudflare Origin Certificate 或 Let's Encrypt）。
- 中继已运行：`relaydrop relay --listen 127.0.0.1:9090 --password <RELAY_PASSWORD>`

## 完整配置

```nginx
# /etc/nginx/sites-available/relay.example.com
map $http_upgrade $connection_upgrade {
    default upgrade;
    ''      close;
}

server {
    listen 443 ssl;
    server_name relay.example.com;

    # Cloudflare Origin Certificate 或 Let's Encrypt
    ssl_certificate     /etc/nginx/certs/relay.example.com.pem;
    ssl_certificate_key /etc/nginx/certs/relay.example.com.key;

    # 仅允许 Cloudflare 回源 IP（可选，增强安全性）
    # 以 Cloudflare 官方 ips-v4 列表为准，变化时需同步更新：
    #   curl -s https://www.cloudflare.com/ips-v4 -o /etc/nginx/cloudflare-ips-v4.txt
    #   for ip in $(cat /etc/nginx/cloudflare-ips-v4.txt); do echo "allow $ip;"; done > /etc/nginx/conf.d/cloudflare-allow.conf
    # include /etc/nginx/conf.d/cloudflare-allow.conf;
    # deny all;

    location /relay {
        # 关键：转发 WebSocket 升级头
        proxy_pass http://127.0.0.1:9090;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;

        # 关键：Cloudflare 空闲超时约 100s，nginx 侧需更长以免提前断开
        proxy_read_timeout 3600s;
        proxy_send_timeout 3600s;
    }

    # 也可完全禁止其它路径
    location / {
        return 404;
    }
}
```

启用：

```bash
ln -sf /etc/nginx/sites-available/relay.example.com /etc/nginx/sites-enabled/
nginx -t && systemctl reload nginx
```

## 防火墙与日志

- **放行 443**：若 VPS 启用了防火墙，需放行 443（Cloudflare 边缘回源走 443）：
  - firewalld：`firewall-cmd --add-port=443/tcp --permanent && firewall-cmd --reload`
  - ufw：`ufw allow 443/tcp`
  - 如启用了上面的「仅允许 Cloudflare 回源 IP」，则 443 实际只对 Cloudflare 网段开放，更安全。
- **日志位置**：访问/错误日志通常在 `/var/log/nginx/access.log`、`/var/log/nginx/error.log`（或在
  `http` 块定义的 `access_log`/`error_log` 路径）。连不通时先查 error.log 是否有 `connect() failed`
  或 `upstream prematurely closed` 等线索。

## 关键配置点

1. **`Upgrade` / `Connection` 头**：WebSocket 握手必须透传，否则连接会在升级阶段失败。
2. **`proxy_read_timeout`**：文件传输可能长时间无应用层「心跳」之外的数据；设为 1 小时级，避免 nginx 提前断连。
   （relaydrop 的 `WsFramed` 也会每 30s 发 WebSocket Ping，进一步保活。）
3. **路径一致**：客户端 `--relay` 必须包含 `/relay`，即 `wss://relay.example.com/relay`。
4. **证书**：用 Cloudflare 控制台签发的 **Origin Certificate**（对其 Authenticated Origin Pulls 友好），
   或使用 Let's Encrypt（需 nginx 能访问 80 做验证，可临时开放）。

## 排错

- **连通性自检**：`nginx -t` 通过后，在能访问 Cloudflare 的网络用 `curl -i -H "Connection: Upgrade" -H "Upgrade: websocket" https://relay.example.com/relay`
  检查是否返回 `101 Switching Protocols`（WebSocket 升级）。非 101 说明 nginx/Cloudflare 配置未就绪。
- 客户端报 `connection refused` / TLS 错误：确认 nginx 在 443 监听且证书有效，域名已橙色云；并确认 443 已在防火墙放行。
- 连接建立后很快断开：检查 `Upgrade`/`Connection` 头与 `proxy_read_timeout`。
- 中继日志显示 `waiting in room` 但不 `paired`：确认发送方与接收方使用 **相同的 `--code` 与 `--password`**。
