# 线协议与加密（开发者向）

本文面向想理解或修改 RelayDrop 传输层/加密层的开发者。所有描述以源码为准：

- 帧抽象与 `Msg` 枚举：`src/framed.rs`
- 密钥派生与 AES-GCM 信封：`src/crypto.rs`

> 本文件只描述当前实现；如需升级为 PAKE / 前向保密，请以 `docs/security.md` 的路线为准并同步更新本文。

## 1. 整体模型

RelayDrop 采用「**中继房间配对 + 双向字节管道**」的核心模型。

```
 sender ──(FramedStream)──► relay room ◄──(FramedStream)──► receiver
        client<->relay 控制通道(relay_key)    peer<->peer 文件(被 file_key 加密)
```

- 中继（`relay.rs`）在「房间」内把两个对等端配对，然后**双向转发原始帧**；它不解析帧内容。
- 发送方与接收方各自与中继建立一条 `FramedStream`；两端的传输方式可不同（一端 `tcp://`、一端 `wss://`），
  因为帧格式与传输无关（见 §3）。
- 房间里只有两个槽位；第三个连接者会收到 `RoomFull`。

## 2. 密钥派生（`src/crypto.rs`）

两把**独立**的 32 字节密钥，均由 `HKDF-SHA256` 派生（`derive_key`，info 固定为 `b"relaydrop-v1"`）：

| 密钥 | 来源 | 保护对象 |
| --- | --- | --- |
| `relay_key` | `HKDF(relay_password)` | 客户端 ↔ 中继的控制通道（`Password` 鉴权、房间名下发） |
| `file_key`  | `HKDF(room_code)` | 发送方 ↔ 接收方的文件分块（`DataChunk` 内容） |

要点：

- 中继持有 `relay_key`，**不持有 `file_key`**（`file_key` 仅由共享 `--code` 的两端派生），因此中继管道里是密文，中继读不到文件。
- `room_code` 即用户传入的 `--code`；房间名由它派生，故两端必须 `--code` 一致才能进同一房间。

## 3. 帧抽象（`FramedStream`）

`FramedStream`（`src/framed.rs`）是一个双向字节流 trait，对协议层隐藏传输差异：

```rust
#[async_trait]
pub trait FramedStream: Send {
    async fn send_bytes(&mut self, data: &[u8]) -> anyhow::Result<()>;
    async fn recv_bytes(&mut self) -> anyhow::Result<Option<Vec<u8>>>; // None = 对端关闭
    fn peer_addr(&self) -> String;
}
```

两种实现：

- **`TcpFramed`**：裸 TCP，每个逻辑帧前加 **4 字节大端长度前缀**。
- **`WsFramed`**：WebSocket，每个逻辑帧是一条 **WebSocket 二进制消息**。后台任务每 **30s** 发 `Ping`
  保活，并自动回 `Pong`（许多公共 WebSocket 代理对空闲连接约 100s 会断开，Ping 保活避免被提前断开）。

中继用 `wrap_incoming` 嗅探入向 TCP：若首行是 `GET ...` 且含 `Upgrade: websocket` 则按 WebSocket 处理，
否则按裸 TCP。客户端用 `connect_client` 按 `--relay` 前缀选择 `tcp://` / `ws://` / `wss://`（TLS 由
`tokio-tungstenite` + `rustls` 完成）。

## 4. 消息枚举（`Msg`）

每条 `Msg` 先由 `postcard` 序列化，再用 `relay_key` 经 `crypto::encrypt` 密封后写入一帧（`send_msg`）；
读取时 `recv_msg` 先解密再 `postcard` 反序列化。

```rust
pub enum Msg {
    Password { enc: Vec<u8> },                 // 客户端->中继：encrypt(relay_key, b"auth")，中继能解密即口令正确
    Ready,                                     // 中继->客户端：握手通过（口令+房间加入后）
    RoomFull,                                  // 中继->客户端：房间已满（第三人）
    RoomJoin { room: String },                 // 客户端->中继：要加入的房间（由 code 派生）
    Manifest { entries: Vec<FileEntry> },       // 发送方->接收方：扁平传输清单（无 is_folder）
    DataChunk(Vec<u8>),                        // 发送方->接收方：当前条目的一个分块（已用 file_key 加密）
    FileEnd,                                   // 发送方->接收方：当前条目结束，接收方校验该文件 SHA-256
    Done,                                      // 发送方->接收方：整次传输结束
    Error { msg: String },                     // 任一方->对端：可读错误
}

pub struct FileEntry {
    pub relpath: String,  // 传输根内的相对路径（恒为相对，不含 ".."）
    pub size: u64,        // 字节数
    pub sha256: String,   // 明文文件的十六进制 SHA-256
}
```

### 一次传输的时序

```
发送方                              中继                              接收方
  |-- Password(enc) -------------->|                                  |
  |-- RoomJoin{room} ------------->|                                  |
  |<---------------- Ready --------|                                  |
  |                                  |<-- Password/ RoomJoin / Ready --|
  |-- Manifest{entries} ---------->|======== 双向转发原始帧 =========>|
  |-- DataChunk* (file 0) -------->|--------------------------------->|
  |-- FileEnd -------------------->|--------------------------------->| 校验 file0 sha256
  |-- DataChunk* (file 1) -------->|--------------------------------->|
  |-- FileEnd -------------------->|--------------------------------->| 校验 file1 sha256
  |-- Done ----------------------->|--------------------------------->| 全部完成
```

- `Manifest.entries` 是**扁平列表**：一次可携带多个文件、多个目录或混合；每个 `FileEntry.relpath`
  相对接收方 `--out`，并保留输入项的顶层名称（文件源为其文件名，目录源为 `dirname/...`）。
- `DataChunk` 的 `Vec<u8>` **已经用 `file_key` 加密**，所以中继即使转发它也看不到明文。
- 接收方对每条 `FileEnd` 校验 SHA-256，不匹配即中止并回报 `Error`。

## 5. 加密信封（`src/crypto.rs`）

- 算法：**AES-256-GCM**（认证加密，带 16 字节 tag）。
- 信封布局：`nonce(12) || ciphertext || tag`。
  - `encrypt`：随机生成 12 字节 nonce，前置到输出；每次加密 nonce 不同。
  - `decrypt`：前 12 字节作 nonce，其余作密文+tag；密钥错误或数据损坏会解密失败。
- 控制通道帧用 `relay_key` 密封，文件分块用 `file_key` 密封（见 `client.rs` 中 `DataChunk` 的封装）。

## 6. 关于 PAKE（口令认证密钥交换）

RelayDrop **省略了 PAKE**，直接由共享口令 `room_code` 派生 `file_key`。这意味着：

- 若中继（你的 VPS）同时知道 `room_code`，它**理论上能解密文件**。
- **假设中继可信**（它是你自己的 VPS）。在此前提下，简化是安全的，且实现更简单。

若中继不可信（第三方托管 / 可能攻陷），应升级为 PAKE 使 `file_key` 不被任何单一方掌握（见 `docs/security.md`）。
