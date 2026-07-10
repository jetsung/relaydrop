//! Transport abstraction + wire protocol.
//!
//! Both the relay and the clients speak the same `Msg` protocol, carried over a
//! `FramedStream`. The framing is transport-agnostic (design.md D4):
//!
//! - `TcpFramed` uses a 4-byte big-endian length prefix on a raw TCP stream.
//! - `WsFramed` uses one WebSocket binary message per frame.
//!
//! The relay pipes raw frames between two peers without caring which transport
//! each peer used, because `recv_bytes`/`send_bytes` hide the difference.

use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex as AsyncMutex;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use crate::crypto;

/// Control / data messages exchanged on a relay channel.
///
/// Every `Msg` is serialized with `postcard` and then sealed with the
/// `relay_key` before being written to the stream (see [`send_msg`]).
#[derive(Serialize, Deserialize, Debug)]
pub enum Msg {
    /// Client -> relay: proves knowledge of the relay password.
    /// The inner bytes are `encrypt(relay_key, b"auth")`; if the relay can
    /// decrypt this, the password was correct.
    Password { enc: Vec<u8> },
    /// Relay -> client: handshake accepted (after password + room join).
    Ready,
    /// Relay -> client: a third peer tried to join an already-full room.
    RoomFull,
    /// Client -> relay: which room (derived from the shared code) to join.
    RoomJoin { room: String },
    /// Sender -> receiver: a manifest describing what will be transferred.
    /// `entries` is a flat list; each file's `relpath` is relative to the
    /// receiver's `--out` and preserves the source's top-level name
    /// (a file source contributes its own base name, a directory source
    /// contributes `dirname/<relative>` for each contained file). This lets a
    /// single transfer carry multiple files, multiple folders, or a mix.
    Manifest { entries: Vec<FileEntry> },
    /// Sender -> receiver: one file chunk for the *current* manifest entry.
    /// The `Vec<u8>` is already sealed with the `file_key`, so the relay
    /// cannot read it.
    DataChunk(Vec<u8>),
    /// Sender -> receiver: the current manifest entry is finished; the
    /// receiver flushes and verifies that file's SHA-256.
    FileEnd,
    /// Sender -> receiver: the whole transfer (all manifest entries) is done.
    Done,
    /// Either side -> other: a human-readable error.
    Error { msg: String },
}

/// One file described by a [`Msg::Manifest`].
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct FileEntry {
    /// Relative path inside the transfer root (always relative, no `..`).
    pub relpath: String,
    /// File size in bytes.
    pub size: u64,
    /// Hex-encoded SHA-256 of the plaintext file.
    pub sha256: String,
}

/// A bidirectional byte stream used by the protocol layer.
#[async_trait]
pub trait FramedStream: Send {
    /// Send one logical frame.
    async fn send_bytes(&mut self, data: &[u8]) -> anyhow::Result<()>;
    /// Receive one logical frame. `None` means the peer closed.
    async fn recv_bytes(&mut self) -> anyhow::Result<Option<Vec<u8>>>;
    /// Peer identifier for logging.
    fn peer_addr(&self) -> String;
}

/// Raw TCP transport with a 4-byte length prefix.
pub struct TcpFramed {
    stream: TcpStream,
    peer: String,
}

impl TcpFramed {
    pub fn new(stream: TcpStream) -> Self {
        let peer = stream
            .peer_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "tcp-peer".to_string());
        Self { stream, peer }
    }
}

#[async_trait]
impl FramedStream for TcpFramed {
    async fn send_bytes(&mut self, data: &[u8]) -> anyhow::Result<()> {
        let len = (data.len() as u32).to_be_bytes();
        self.stream.write_all(&len).await?;
        self.stream.write_all(data).await?;
        self.stream.flush().await?;
        Ok(())
    }

    async fn recv_bytes(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        let mut len_buf = [0u8; 4];
        match self.stream.read_exact(&mut len_buf).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        }
        let len = u32::from_be_bytes(len_buf) as usize;
        let mut buf = vec![0u8; len];
        self.stream.read_exact(&mut buf).await?;
        Ok(Some(buf))
    }

    fn peer_addr(&self) -> String {
        self.peer.clone()
    }
}

/// WebSocket transport. The stream is split into a write half (shared, so a
/// background keep-alive task can send pings) and a read half.
pub struct WsFramed<S> {
    write: std::sync::Arc<AsyncMutex<futures_util::stream::SplitSink<WebSocketStream<S>, Message>>>,
    read: futures_util::stream::SplitStream<WebSocketStream<S>>,
    peer: String,
    _keepalive: tokio::task::JoinHandle<()>,
}

impl<S> WsFramed<S>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin + Send + 'static,
{
    pub fn new(ws: WebSocketStream<S>, peer: String) -> Self {
        let (write, read) = ws.split();
        let write = std::sync::Arc::new(AsyncMutex::new(write));
        let write_clone = write.clone();
        let keepalive = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let mut w = write_clone.lock().await;
                if w.send(Message::Ping(vec![].into())).await.is_err() {
                    break;
                }
            }
        });
        Self {
            write,
            read,
            peer,
            _keepalive: keepalive,
        }
    }
}

#[async_trait]
impl<S> FramedStream for WsFramed<S>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin + Send + 'static,
{
    async fn send_bytes(&mut self, data: &[u8]) -> anyhow::Result<()> {
        let mut w = self.write.lock().await;
        w.send(Message::Binary(data.to_vec().into())).await?;
        Ok(())
    }

    async fn recv_bytes(&mut self) -> anyhow::Result<Option<Vec<u8>>> {
        loop {
            match self.read.next().await {
                None => return Ok(None),
                Some(Err(e)) => return Err(e.into()),
                Some(Ok(Message::Binary(b))) => return Ok(Some(b.to_vec())),
                Some(Ok(Message::Text(t))) => return Ok(Some(t.as_bytes().to_vec())),
                Some(Ok(Message::Ping(p))) => {
                    // Answer keep-alives so the connection (which may have an
                    // idle timeout) stays open.
                    let mut w = self.write.lock().await;
                    let _ = w.send(Message::Pong(p)).await;
                }
                Some(Ok(Message::Pong(_))) => {}
                Some(Ok(Message::Close(_))) => return Ok(None),
                Some(Ok(_)) => {}
            }
        }
    }

    fn peer_addr(&self) -> String {
        self.peer.clone()
    }
}

/// Serialize + seal a `Msg` and write it as one frame.
pub async fn send_msg(
    stream: &mut dyn FramedStream,
    key: &[u8; 32],
    msg: &Msg,
) -> anyhow::Result<()> {
    let bytes = postcard::to_allocvec(msg)?;
    let sealed = crypto::encrypt(key, &bytes);
    stream.send_bytes(&sealed).await
}

/// Read + open one frame and deserialize a `Msg`.
pub async fn recv_msg(
    stream: &mut dyn FramedStream,
    key: &[u8; 32],
) -> anyhow::Result<Option<Msg>> {
    match stream.recv_bytes().await? {
        None => Ok(None),
        Some(blob) => {
            let plain = crypto::decrypt(key, &blob)?;
            let msg: Msg = postcard::from_bytes(&plain)?;
            Ok(Some(msg))
        }
    }
}

/// Accept an already-connected TCP stream and wrap it as a `FramedStream`,
/// sniffing whether the peer is speaking WebSocket (HTTP upgrade) or raw TCP.
pub async fn wrap_incoming(stream: TcpStream) -> anyhow::Result<Box<dyn FramedStream>> {
    let mut peek_buf = [0u8; 4096];
    let n = stream.peek(&mut peek_buf).await?;
    let head = String::from_utf8_lossy(&peek_buf[..n]);
    let is_ws = head.starts_with("GET ")
        && head.to_ascii_lowercase().contains("upgrade: websocket");

    if is_ws {
        let peer = stream
            .peer_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "ws-peer".to_string());
        let ws = tokio_tungstenite::accept_async(stream).await?;
        Ok(Box::new(WsFramed::new(ws, peer)))
    } else {
        Ok(Box::new(TcpFramed::new(stream)))
    }
}

/// Connect to a relay from a client, returning a `FramedStream`.
/// Supports `tcp://` (raw), `ws://` (plain WebSocket) and `wss://`
/// (TLS WebSocket, when a reverse proxy terminates TLS in front of the relay).
pub async fn connect_client(relay_url: &str) -> anyhow::Result<Box<dyn FramedStream>> {
    // 归一化协议头：
    // - 已带 tcp:// / ws:// / wss:// 的原样保留；
    // - 含 "://" 但非上述三种（如 http://）视为未知 scheme，直接报错；
    // - 其余（裸 host:port）按 tcp:// 处理，方便只写 `127.0.0.1:9009`。
    let relay_url: String = if relay_url.starts_with("tcp://")
        || relay_url.starts_with("ws://")
        || relay_url.starts_with("wss://")
    {
        relay_url.to_string()
    } else if relay_url.contains("://") {
        anyhow::bail!("relay URL must start with tcp://, ws:// or wss:// (got: {relay_url})")
    } else {
        format!("tcp://{relay_url}")
    };

    if relay_url.starts_with("wss://") || relay_url.starts_with("ws://") {
        let (ws, _resp) = tokio_tungstenite::connect_async(&relay_url).await?;
        Ok(Box::new(WsFramed::new(ws, relay_url)))
    } else if let Some(addr) = relay_url.strip_prefix("tcp://") {
        let tcp = TcpStream::connect(addr).await?;
        Ok(Box::new(TcpFramed::new(tcp)))
    } else {
        anyhow::bail!("relay URL must start with tcp://, ws:// or wss:// (got: {relay_url})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_manifest_postcard() {
        let entries = vec![
            FileEntry { relpath: "Cargo.lock".into(), size: 31205, sha256: "abc".into() },
            FileEntry { relpath: "a/b/x.txt".into(), size: 7, sha256: "def".into() },
        ];
        let m = Msg::Manifest { entries: entries.clone() };
        let bytes = postcard::to_allocvec(&m).expect("serialize");
        let back: Msg = postcard::from_bytes(&bytes).expect("deserialize");
        match back {
            Msg::Manifest { entries: e } => assert_eq!(e, entries),
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
