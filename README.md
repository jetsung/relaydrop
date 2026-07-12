# relaydrop

一个文件中继工具，核心特性是 **支持通过 WebSocket over TLS（`wss://`）中转**，让发送方与接收方可以经过任意 TLS 终结的反向代理或隧道访问中继，而中继本身不持有证书、不占用 443。

## 为什么

「房间配对 + 双向字节管道」协议默认承载在 TCP 上（中继默认 `:9090`）。relaydrop 额外把同一套协议承载到 **WebSocket over TLS** 之上，让发送方与接收方可以经过任意 TLS 终结的反向代理（如 nginx）或隧道（如 cloudflared）访问中继，而中继本身不持有证书、不占用 443。

## 架构

```
 sender ─┐                                    ┌─ receiver
         │   relay channel (relay_key)        │
         ├──►  relay (room)  ◄──pipe──►  relay ◄──┤
         │     VPS, fronted by a TLS terminator   │
 wss://relay.example.com  (nginx TLS 终结 + 代理 / cloudflared tunnel)
```

- **中继（relay）**：在「房间（room）」内把发送方与接收方配对，并在两端之间双向转发字节。本身 **不持有 TLS 证书、不占用 443**，由 nginx / cloudflared 在前端终结 TLS。
- **两把密钥（见 `src/crypto.rs`）**：
  - `relay_key = HKDF(relay_password)`：保护客户端 ↔ 中继的控制通道（口令校验、房间名）。
  - `file_key  = HKDF(room_code)`：保护发送方 ↔ 接收方的文件内容。**中继不知道 `file_key`，因此无法读取文件明文。**
- **传输无关帧（见 `src/framed.rs`）**：`TcpFramed`（裸 TCP，4 字节长度前缀）与 `WsFramed`（WebSocket 二进制消息）实现同一个 `FramedStream` trait，中继管道无需关心每端用的是哪种传输。

> 注：用「由共享口令派生密钥」而非完整的 PAKE（口令认证密钥交换）；两端本就输入同一共享口令，故可直接派生。安全权衡见 [docs/security.md](docs/security.md)。

## 构建

```bash
cargo build --release
# 产物: target/release/relaydrop
```

依赖：tokio、tokio-tungstenite（rustls TLS）、aes-gcm、hkdf、sha2、clap、postcard、serde、rand、anyhow、futures-util、async-trait、hex、base64。

## 快速上手（本机直连，验证功能）

开三个终端：

```bash
# 1) 中继（本机，明文 ws/tcp 混用，监听 127.0.0.1:9090）
./target/release/relaydrop relay --listen 127.0.0.1:9090 --password SECRET

# 2) 发送方（无需指定 --code：自动生成随机密钥并打印接收命令）
./target/release/relaydrop send --relay tcp://127.0.0.1:9090 --password SECRET ./big.iso
# 终端会打印，例如：
#   On the other computer run:
#     relaydrop receive --relay tcp://127.0.0.1:9090 --code <随机> --password SECRET --out .

# 3) 接收方：直接粘贴上面打印的命令即可（发送方先发起，接收方后加入）
```

传输完成后接收方会校验每个文件的 SHA-256。

### 文件夹传输

`send` 接受文件**或目录**。目录会被递归打包为清单（相对路径、大小、逐文件哈希），
接收方按相对路径重建目录结构，逐文件校验：

```bash
./target/release/relaydrop send --relay tcp://127.0.0.1:9090 --password SECRET ./myfolder
# 对端粘贴打印出的 receive 命令，得到完整 myfolder/ 目录树
```

符号链接与非常规文件会被跳过（并在日志提示）。

## 生产部署（TLS WebSocket）

生产部署让中继只监听本地，TLS 与 WebSocket 代理交给前端反向代理（如 nginx）或隧道（如 cloudflared）终结：

```bash
# 中继（仅本地，明文 ws）
./target/release/relaydrop relay --listen 127.0.0.1:9090 --password SECRET
```

```bash
# 客户端通过前端域名连接（wss://）
./target/release/relaydrop send    --relay wss://relay.example.com/relay --code MYCODE --password SECRET --file ./big.iso
./target/release/relaydrop receive --relay wss://relay.example.com/relay --code MYCODE --password SECRET --out ./downloads
```

完整部署与排错见下方文档。

## CLI

| 子命令 | 作用 |
| --- | --- |
| `relay`   | 运行中继。`--listen`（默认 `127.0.0.1:9090`）、`--password`、`--ttl`（房间超时秒） |
| `send`    | 发送文件或目录。`--relay`、`--code`（省略则随机生成）、`--password`、`--file`/位置参数路径 |
| `receive` | 接收文件或目录。`--relay`、`--code`、`--password`、`--out` |

`send` 省略 `--code` 时会生成随机密钥并打印一条可复制粘贴的 `receive` 命令（含 `--code` 与 `--password`）。
`--relay` 支持三种形式：`tcp://host:port`（裸 TCP）、`ws://host:port/path`（明文 WebSocket）、`wss://host/path`（经 TLS 终结的 WebSocket）；**省略协议头（形如 `127.0.0.1:9090`）时默认按 `tcp://` 处理**。以 `wss://` 开头即自动走加密 WebSocket。

> **环境变量**：所有命令行参数也都可通过环境变量设置——`RELAYDROP_` 前缀加参数长名大写，例如 `RELAYDROP_RELAY`、`RELAYDROP_PASSWORD`、`RELAYDROP_CODE`、`RELAYDROP_PATH`、`RELAYDROP_OUT`。优先级为 **命令行 > 环境变量 > 默认值**；完整变量表与 Docker 用法见 [docs/docker.md](docs/docker.md)。

## 文档

- [docs/install.md](docs/install.md) — VPS 中继安装与三种部署方式
- [docs/nginx.md](docs/nginx.md) — nginx 反向代理完整配置（含回源 IP 白名单、防火墙）
- [docs/cloudflare.md](docs/cloudflare.md) — Cloudflare 侧配置
- [docs/cloudflared.md](docs/cloudflared.md) — cloudflared 隧道（命名隧道 + systemd 守护）
- [docs/usage.md](docs/usage.md) — 客户端完整用法、示例与 FAQ
- [docs/docker.md](docs/docker.md) — 环境变量与 Docker 镜像/运行用法
- [docs/security.md](docs/security.md) — 安全提示与方案权衡
- [docs/protocol.md](docs/protocol.md) — 开发者向：线协议、加密信封与密钥派生

> 术语约定：发送方省略 `--code` 时**自动生成随机传输密钥**并打印可粘贴的 `receive` 命令；
> `--password` 是**运维预先确定的固定常量**，原样嵌入命令供接收方向中继鉴权。详见各文档。

## 安全提示

- 使用高熵共享口令（`--code`）：文件密钥由它派生，弱口令可被暴力。
- 中继口令（`--password`）用于客户端 ↔ 中继鉴权，请与 `code` 不同且足够强。
- 默认假设中继（你的 VPS）可信；若要防御恶意/被攻陷的中继，需升级为完整 PAKE（见 docs/security.md）。

## 常见问题

- **relaydrop 与其他 relay 工具互通吗？** 不互通。relaydrop 是独立实现，协议与口令/密钥模型均不同；发送方与接收方都须使用 relaydrop。
- **支持多大文件 / 断点续传吗？** 文件大小仅受磁盘限制，按 64KB 流式分块；不支持断点续传、压缩、多人。
- **中继一定要用 `wss://` 吗？** 不是。中继可达时用 `tcp://`/`ws://` 直连即可；只有当客户端与中继之间存在 TLS 终结层（反向代理 / 隧道）时，才使用 `wss://`。
- 更多排错与示例见 [docs/usage.md](docs/usage.md)。

## Docker 镜像

> **版本：** `latest`, `dev`(GHCR only), <`TAG`>

| Registry                                                                                   | Image                                                  |
| ------------------------------------------------------------------------------------------ | ------------------------------------------------------ |
| [**Docker Hub**](https://hub.docker.com/r/jetsung/relaydrop/)                                | `jetsung/relaydrop`                                    |
| [**GitHub Container Registry**](https://ghcr.io/jetsung/relaydrop) | `ghcr.io/jetsung/relaydrop`                            |
| **Tencent Cloud Container Registry（SG）**                                                       | `sgccr.ccs.tencentyun.com/jetsung/relaydrop`             |
| **Aliyun Container Registry（GZ）**                                                              | `registry.cn-guangzhou.aliyuncs.com/jetsung/relaydrop` |

## 仓库镜像

[MyCode](https://git.jetsung.com/jetsung/relaydrop) ● [AtomGit](https://atomgit.com/jetsung/relaydrop) ● [GitHub](https://github.com/jetsung/relaydrop)
