# Docker 使用

本页说明如何使用 Docker 镜像运行中继（relay）、发送方（send）与接收方（receive）。环境变量的完整变量表、优先级与 systemd 集成见 [env.md](env.md)（唯一事实来源）。

## Docker 镜像

> **版本：** `latest`, `dev`(GHCR only), <`TAG`>

| Registry                                                                                   | Image                                                  |
| ------------------------------------------------------------------------------------------ | ------------------------------------------------------ |
| [**Docker Hub**](https://hub.docker.com/r/jetsung/relaydrop/)                                | `jetsung/relaydrop`                                    |
| [**GitHub Container Registry**](https://ghcr.io/jetsung/relaydrop) | `ghcr.io/jetsung/relaydrop`                            |
| **Tencent Cloud Container Registry（SG）**                                                       | `sgccr.ccs.tencentyun.com/jetsung/relaydrop`             |
| **Aliyun Container Registry（GZ）**                                                              | `registry.cn-guangzhou.aliyuncs.com/jetsung/relaydrop` |

> 注：镜像默认即监听 `0.0.0.0:9090`——该默认值来自 `docker/Dockerfile` 的 `ENV RELAYDROP_LISTEN=0.0.0.0:9090`，而容器 `CMD` 仅为 `["relay"]`。无需额外参数即可作为可达中继；TLS 仍由前端反向代理 / 隧道终结。如需只监听本地，可用 `-e RELAYDROP_LISTEN=127.0.0.1:9090` 覆盖。

## Docker Compose 部署

仓库内的 `docker/compose.yaml` 通过 `env_file` 读取宿主机上的 `./.env`（可选，`required: false`）来注入 `RELAYDROP_*` 环境变量：

```yaml
services:
  relaydrop:
    container_name: relaydrop
    env_file:
      - path: ./.env
        required: false
    hostname: relaydrop
    image: ghcr.io/jetsung/relaydrop:latest
    ports:
      - 9090:9090
    restart: unless-stopped
```

启动：

```bash
docker compose -f docker/compose.yaml up -d
```

环境变量写在 `./.env` 里（一行一个 `KEY=VALUE`），例如只跑本地监听的中继：

```bash
# .env
RELAYDROP_LISTEN=127.0.0.1:9090
RELAYDROP_PASSWORD=SECRET
```

`RELAYDROP_*` 变量名与本页顶部「环境变量」小节完全一致；除口令外，`RELAYDROP_LISTEN`、`RELAYDROP_TTL` 等也都能通过 `.env` 配置。这与下方 `docker run -e RELAYDROP_*` 是同一套变量，区别只在传入方式（文件 vs 命令行）。

## Docker 运行示例

所有命令行参数都可用 `-e RELAYDROP_*` 传入；下表给出三个子命令的容器化命令。

### 中继（relay）

```bash
docker run -d --name relaydrop-relay \
  -p 127.0.0.1:9090:9090 \
  -e RELAYDROP_PASSWORD=SECRET \
  jetsung/relaydrop
```

> 镜像默认启动 `relay` 并监听 `0.0.0.0:9090`。端口映射建议绑定到 `127.0.0.1`（如 `-p 127.0.0.1:9090:9090`），仅本机 / 同主机的前端代理（nginx / cloudflared）可达，避免直接暴露公网；需要远程直连再开放到 `0.0.0.0`。

### 发送方（send）

```bash
# 发送当前目录下的 ./big.iso
docker run --rm \
  -v "$(pwd):/data" \
  -e RELAYDROP_RELAY=wss://relay.example.com/relay \
  -e RELAYDROP_PASSWORD=SECRET \
  -e RELAYDROP_CODE=MYCODE \
  jetsung/relaydrop send --file /data/big.iso
```

- 用 `-v` 把本地目录挂进容器（示例挂到 `/data`），路径参数用容器内路径。
- 省略 `-e RELAYDROP_CODE` 时，`send` 会随机生成并打印一条 `receive` 命令。

### 接收方（receive）

```bash
docker run --rm \
  -v "$(pwd)/downloads:/out" \
  -e RELAYDROP_RELAY=wss://relay.example.com/relay \
  -e RELAYDROP_PASSWORD=SECRET \
  -e RELAYDROP_CODE=MYCODE \
  -e RELAYDROP_OUT=/out \
  jetsung/relaydrop receive
```

- `-e RELAYDROP_OUT=/out` 与挂载点保持一致，文件会落到宿主的 `./downloads`。
