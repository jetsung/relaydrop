# 客户端用法

发送方与接收方通过相同的共享口令（`--code`）在 relay 的同一个房间里配对，文件内容经 `file_key` 加密，
中继只能看到密文。

## 通用参数

| 参数 | 说明 |
| --- | --- |
| `--relay`   | 中继地址，支持 `tcp://host:port`、`ws://host:port/path`、`wss://host/path`；**省略协议头（`host:port`）时默认按 `tcp://` 处理** |
| `--code`    | 共享口令 / 房间名。**发送方与接收方必须一致** |
| `--password`| 中继口令，须与中继 `--password` 一致 |

## 环境变量

除命令行参数外，每个参数也都可用 `RELAYDROP_` 前缀的环境变量设置（如 `RELAYDROP_RELAY`、`RELAYDROP_PASSWORD`）。

> 环境变量的完整变量表、优先级与 systemd 集成见 [env.md](env.md)（唯一事实来源）；Docker / compose 部署中的环境变量用法见 [docker.md](docker.md)。

## 发送文件或文件夹（一条命令接收）

发送方省略 `--code` 时会**自动生成随机密钥**并打印一条可复制粘贴的接收命令；发送方先发起，
接收方后加入。中继口令 `--password` 是**事先确定的固定常量**，会原样嵌入打印命令，使接收方也能向中继鉴权。

```bash
# 发送方
relaydrop send --relay wss://relay.example.com/relay --password SECRET ./big.iso
# 终端打印：
#   On the other computer run:
#     relaydrop receive --relay wss://relay.example.com/relay --code <随机> --password SECRET --out .

# 接收方：直接粘贴上面打印的命令
```

- 流程：生成随机 `code` → 打印接收命令 → 连接 → 鉴权 → 加入房间 → 发送清单（路径/大小/逐文件哈希）
  → 逐文件分块（64KB）加密发送 → 每个文件结束发 `FileEnd` → 全部结束发 `Done`。
- 终端会显示已发送字节进度（按当前文件）。
- 若需固定 `code`（脚本/自动化），显式传 `--code` 即可，打印命令会使用它。

## 文件夹传输与多源发送

`send` 支持任意数量的源——多个文件、多个目录、或文件与目录的混合，全部在一次传输里完成：

```bash
# 多个文件 + 一个目录，混发
relaydrop send --relay wss://relay.example.com/relay --password SECRET ./a.txt ./b.log ./myfolder

# 也可用可重复的 --file 指定（与位置参数合并）
relaydrop send --relay wss://relay.example.com/relay --password SECRET --file a.txt --file b.log
```

规则：

- 每个输入项的**顶层名称会被保留**在接收方的 `--out` 下：文件 `a.txt` 直接落在 `--out/a.txt`；
  目录 `myfolder/` 展开为 `--out/myfolder/...`（目录名作为前缀）。
- 发送方把所有源**扁平化**为一份清单（相对路径、大小、逐文件 SHA-256），按路径排序后发送；
  接收方按相对路径重建目录树（自动创建子目录），逐文件解密写入并校验 SHA-256。
- **符号链接与非常规文件会被跳过**（日志中提示），不跟随、不传输。
- **同名冲突会报错**：若不同输入产生相同顶层名（如两个不同父目录下都叫 `a.txt`，或两个同名目录），
  发送方在发送前检测并报错 `duplicate entry name: ...`，不会发出任何数据。

## 接收文件

```bash
relaydrop receive \
  --relay wss://relay.example.com/relay \
  --code MYCODE \
  --password SECRET \
  --out ./downloads
```

- 接收方先启动也没关系（房间会等待另一个对等端）。
- 先收到清单（`Manifest`），再按条目流式接收并解密分块、写入本地；每文件结束时校验其 SHA-256。
- 路径穿越被阻止：仅取相对路径的正常组件，丢弃 `..`/绝对路径。
- 所有条目收齐并收到 `Done` 后报告成功；任一文件哈希不匹配即报错中止。

## 三种 `--relay` 模式

| 模式 | 场景 | 示例 |
| --- | --- | --- |
| `tcp://` | 中继可达的裸 TCP（同内网/直连） | `tcp://192.168.1.10:9090` |
| `ws://`  | 明文 WebSocket（直连中继，已绕过 TLS） | `ws://relay.example.com:9090/relay` |
| `wss://` | **经 TLS 终结的 WebSocket（生产部署）** | `wss://relay.example.com/relay` |

以 `wss://` 开头会自动走加密 WebSocket（由 tokio-tungstenite + rustls 完成 TLS）。

不带协议头时（例如 `--relay 192.168.1.10:9090` 或 `RELAYDROP_RELAY=127.0.0.1:9090`），客户端会自动补上 `tcp://` 再连接，因此裸 `host:port` 等价于 `tcp://host:port`。其它无法识别的协议头（如 `http://`）仍会报错，提示必须使用 `tcp://` / `ws://` / `wss://`。

## 完整示例（本机直连验证）

```bash
# 终端 1：中继
relaydrop relay --listen 127.0.0.1:9090 --password SECRET

# 终端 2：接收
relaydrop receive --relay tcp://127.0.0.1:9090 --code ABC --password SECRET --out ./dl

# 终端 3：发送
relaydrop send --relay tcp://127.0.0.1:9090 --code ABC --password SECRET --file ./photo.zip
```

## 跨网络拓扑示例（客户端与中继跨网络）

典型场景：客户端与中继不在同一网络（如中继在远端 VPS，发送方与其同内网，接收方在另一网络）。两端都通过 `wss://` 经前端的 TLS 终结层连到
那台 VPS 上的明文中继，文件在管道里是密文。

**中继所在 VPS**：

```bash
# 中继仅本地明文监听，TLS 由 nginx/cloudflared 在前端终结
relaydrop relay --listen 127.0.0.1:9090 --password SECRET
# 另起：nginx 反向代理 或 cloudflared 隧道，把 wss://relay.example.com 暴露出去
# （部署见 docs/nginx.md / docs/cloudflared.md，前端域名指向中继）
```

**发送方（与中继同内网/可达）**：

```bash
relaydrop send --relay tcp://127.0.0.1:9090 --password SECRET ./myfolder
# 终端打印：
#   On the other computer run:
#     relaydrop receive --relay wss://relay.example.com --code <随机> --password SECRET --out .
```

**接收方（可访问前端域名）**：直接粘贴上面打印的命令（注意 `--relay` 已是 `wss://relay.example.com`）：

```bash
relaydrop receive --relay wss://relay.example.com --code <随机> --password SECRET --out .
```

要点：

- 发送方 `--relay` 用 `tcp://` 连本地中继（不经过公网）；接收方 `--relay` 用 `wss://` 经前端 TLS 终结层。
- 同一 `--code`/`--password` 让两端进入中继同一房间；文件夹会被递归打包、按相对路径重建、逐文件校验。
- 只要前端 TLS 终结层可达，接收方即可完成接收。

## 常见问题

- **`relay room is full`**：同一 `--code` 已有两人在房间。换一个 `--code` 或等待房间超时（默认 300s）。
- **`integrity check failed`**：文件损坏或 `--code` 不一致导致密钥不同。确认两端 `--code` 完全一致。
- **连接被拒绝 / TLS 错误**：检查 `--relay` 域名与前端证书状态（见 [nginx.md](nginx.md) / [cloudflare.md](cloudflare.md)）。
- **RelayDrop 与其他 relay 工具能互通吗？**：**不能**。RelayDrop 是独立的实现，线协议、口令/密钥模型（`HKDF` 直接派生，无 PAKE）均不同。发送方与接收方**必须都用 relaydrop**。
- **支持多大的文件？**：单文件/文件夹总大小仅受磁盘与中继内存限制；传输按 64KB 分块流式进行，不会整文件入内存。没有单文件大小上限（远超 GB 亦可，只要两端磁盘足够）。
- **支持断点续传 / 压缩 / 多对多吗？**：**均不支持**。一次传输为单房间、单发送方对单接收方、整文件重传；中断需重新发起。需要这些能力属后续增强范围。
- **为什么要用 `wss://`？** 当客户端与中继之间存在 TLS 终结层（反向代理 / 隧道）时，`wss://` 让客户端经该层访问中继；若中继本身可达，用 `tcp://`/`ws://` 直连即可，无需 TLS 层。
- **房间超时（TTL）怎么算？** 房间在未被配对时按 `--ttl`（默认 300s）存活，超时自动清理；配对成功后传输期间不会被清理。发送方先连上等待时，若超过 TTL 仍未有接收方加入，需重发以重新建房间。
