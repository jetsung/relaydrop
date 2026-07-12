# 环境变量与 Docker 使用

本页集中说明 relaydrop 的环境变量配置，以及如何使用 Docker 镜像运行中继（relay）、发送方（send）与接收方（receive）。

## 环境变量

除命令行参数外，每个参数的值也都可以通过环境变量设置。变量名为 `RELAYDROP_` 前缀 + 参数长名大写：

| 子命令 | 参数 | 环境变量 |
| --- | --- | --- |
| `relay`   | `--listen` | `RELAYDROP_LISTEN` |
| `relay`   | `--password` | `RELAYDROP_PASSWORD` |
| `relay`   | `--ttl` | `RELAYDROP_TTL` |
| `send`    | `--relay` | `RELAYDROP_RELAY` |
| `send`    | `--code` | `RELAYDROP_CODE` |
| `send`    | `--password` | `RELAYDROP_PASSWORD` |
| `send`    | 位置参数 `path` | `RELAYDROP_PATH` |
| `receive` | `--relay` | `RELAYDROP_RELAY` |
| `receive` | `--code` | `RELAYDROP_CODE` |
| `receive` | `--password` | `RELAYDROP_PASSWORD` |
| `receive` | `--out` | `RELAYDROP_OUT` |

**优先级**：显式命令行参数 **>** 环境变量 **>** 默认值。

```bash
# 例：用环境变量提供中继口令，命令行只写必要项
export RELAYDROP_PASSWORD=SECRET
export RELAYDROP_RELAY=wss://relay.example.com
relaydrop send --code MYCODE ./big.iso        # 自动读 RELAYDROP_PASSWORD / RELAYDROP_RELAY
```

- 命令行上显式写的参数永远覆盖同名环境变量；两者都未给时才用默认值（或 `send` 的「省略 `--code` 则随机生成」）。
- 若环境里残留 `RELAYDROP_CODE`，`send` 会使用它而不是随机生成——这是预期行为，注意清理不需要的环境变量。
- 与 systemd `Environment=` / `EnvironmentFile=` 天然兼容：systemd 注入的环境变量会被进程继承，clap 直接读取（见 [install.md](install.md)）。

### systemd 集成示例

```ini
# /etc/systemd/system/relaydrop.service
[Unit]
Description=relaydrop relay
After=network.target

[Service]
User=relaydrop
Environment=RELAYDROP_PASSWORD=SECRET
Environment=RELAYDROP_LISTEN=127.0.0.1:9090
ExecStart=/usr/local/bin/relaydrop relay
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

也可用 `EnvironmentFile=` 从文件加载（一行一个 `KEY=VALUE`）：

```ini
EnvironmentFile=/etc/relaydrop/env
```

## Docker 镜像

> **版本：** `latest`, `dev`(GHCR only), <`TAG`>

| Registry                                                                                   | Image                                                  |
| ------------------------------------------------------------------------------------------ | ------------------------------------------------------ |
| [**Docker Hub**](https://hub.docker.com/r/jetsung/relaydrop/)                                | `jetsung/relaydrop`                                    |
| [**GitHub Container Registry**](https://ghcr.io/jetsung/relaydrop) | `ghcr.io/jetsung/relaydrop`                            |
| **Tencent Cloud Container Registry（SG）**                                                       | `sgccr.ccs.tencentyun.com/jetsung/relaydrop`             |
| **Aliyun Container Registry（GZ）**                                                              | `registry.cn-guangzhou.aliyuncs.com/jetsung/relaydrop` |

> 注：容器默认即以 `0.0.0.0:9090` 启动中继（`CMD ["relay","--listen","0.0.0.0:9090"]`），无需额外参数即可作为可达中继；TLS 仍由前端反向代理 / 隧道终结。如需只监听本地，可用 `-e RELAYDROP_LISTEN=127.0.0.1:9090` 覆盖。

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
