# 环境变量

relaydrop 的所有命令行参数也都可以通过环境变量设置。变量名为 `RELAYDROP_` 前缀 + 参数长名大写，例如 `RELAYDROP_RELAY`、`RELAYDROP_PASSWORD`。本文是环境变量的**唯一事实来源**；Docker 与 compose 部署见 [docker.md](docker.md)，客户端完整用法见 [usage.md](usage.md)。

## 变量表

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

## systemd 集成

与 systemd `Environment=` / `EnvironmentFile=` 天然兼容：systemd 注入的环境变量会被进程继承，clap 直接读取（部署示例见 [install.md](install.md)）。

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

## 与 Docker / compose 的关系

容器部署（`docker run -e` 或 compose 的 `./.env`）用的也是上表同一套 `RELAYDROP_*` 变量，区别只在传入方式（命令行 / 文件）。compose 的 `.env` 示例与容器运行示例见 [docker.md](docker.md)。
