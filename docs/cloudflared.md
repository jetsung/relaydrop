# cloudflared 隧道部署

当 VPS 的 443/80 被占用、或你**不想开放任何公网端口**时，用 `cloudflared` 让 Cloudflare 直接把
流量隧道到本地中继。`cloudflared` 在 Cloudflare 边缘与 VPS 之间建立一条出向连接，VPS 无需监听公网端口。

中继仍然是**明文**监听本地端口，TLS 在 Cloudflare 边缘终结：

```bash
relaydrop relay --listen 127.0.0.1:9090 --password <RELAY_PASSWORD>
```

## 两种隧道

### A. 临时隧道（快速验证，不推荐生产）

```bash
cloudflared tunnel --url ws://127.0.0.1:9090
```

`cloudflared` 会分配一个随机的 `*.trycloudflare.com` 域名（如 `https://<random>.trycloudflare.com`）。
客户端用：

```bash
relaydrop send    --relay wss://<random>.trycloudflare.com --code MYCODE --password SECRET --file ./big.iso
relaydrop receive --relay wss://<random>.trycloudflare.com --code MYCODE --password SECRET --out ./dl
```

> 缺点：域名每次启动都变、不可记忆、无法设置访问策略，仅适合临时测试。

### B. 命名隧道（持久化，推荐生产）

1. 登录并授权 cloudflared 关联你的 Cloudflare 账号：

   ```bash
   cloudflared tunnel login
   ```

2. 创建命名隧道（只需一次）：

   ```bash
   cloudflared tunnel create relaydrop
   # 输出隧道 ID 与凭据路径，如 /root/.cloudflared/<id>.json
   ```

3. 编写配置文件（把 `relay.example.com` 换成你的域名，且已在 Cloudflare 设为橙色云）：

   ```yaml
   # /etc/cloudflared/config.yml
   tunnel: relaydrop
   credentials-file: /root/.cloudflared/<id>.json

   ingress:
     - hostname: relay.example.com
       service: ws://127.0.0.1:9090
     - service: http_status:404
   ```

   > `service` 用 `ws://` 前缀，cloudflared 会自动处理 WebSocket 升级。

4. 启动并设为守护进程（systemd）：

   ```ini
   # /etc/systemd/system/cloudflared.service
   [Unit]
   Description=cloudflared tunnel for relaydrop
   After=network.target

   [Service]
   ExecStart=/usr/local/bin/cloudflared tunnel run
   Restart=on-failure
   User=cloudflared

   [Install]
   WantedBy=multi-user.target
   ```

   ```bash
   systemctl enable --now cloudflared
   ```

5. 客户端连接（稳定域名）：

   ```bash
   relaydrop send    --relay wss://relay.example.com --code MYCODE --password SECRET --file ./big.iso
   relaydrop receive --relay wss://relay.example.com --code MYCODE --password SECRET --out ./dl
   ```

## 关键配置点

- **`ws://` 前缀**：`ingress` 的 `service` 写成 `ws://127.0.0.1:9090`，cloudflared 会透明代理 WebSocket；
  不要写成 `http://`（虽也能升级，但 `ws://` 语义更明确）。
- **空闲超时**：Cloudflare 对 WebSocket 约 100s 无数据会断；relaydrop 的 `WsFramed` 每 30s 发 Ping 保活，
  配对等待与长传输都不会触发空闲断开。
- **客户端 `--relay` 不含 `/relay` 路径**：cloudflared 按 `hostname` 路由（不像 nginx 用 `location /relay`），
  所以直接写 `wss://relay.example.com` 即可。

## 排错

- 客户端报 `connection refused` / TLS 错误：确认域名为橙色云、隧道 `systemctl status cloudflared` 为 active。
- 能连上但很快断开：检查 `ingress` 的 `service` 是否指向正确的本地端口（9090），以及 `proxy_read_timeout` 无需在此配置（cloudflared 默认长连接）。
- 中继日志 `waiting in room` 但不 `paired`：确认发送方与接收方 `--code`/`--password` 完全一致。

## 与 nginx 方案对比

| 维度 | nginx 反向代理 | cloudflared 隧道 |
| --- | --- | --- |
| 需开放公网端口 | 需 443 | 无需任何公网端口 |
| 域名 | 自有域名（橙色云） | 自有域名（命名隧道）或临时 `*.trycloudflare.com` |
| 运维 | 维护 nginx 配置 | 维护 cloudflared + 隧道凭据 |
| 适用 | 443 已被 nginx 占用时复用 | 不想动 nginx / 无开放端口时 |

两种方式对 relaydrop 而言**等价**：客户端都通过 `wss://` 经 Cloudflare 边缘连到本地明文中继。
