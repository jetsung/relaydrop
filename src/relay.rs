//! Relay server: pairs two peers in a room and pipes bytes between them.
//!
//! Protocol (design.md / relay-protocol spec):
//! 1. Peer connects, sends `Password` then `RoomJoin`.
//! 2. Relay decrypts those two messages with `relay_key` (wrong password =>
//!    decrypt fails => connection dropped). The second message reveals the room.
//! 3. First peer to a room waits; second peer pairs and the relay pipes all
//!    further frames bidirectionally. The relay never interprets file content
//!    (it is sealed with `file_key`, unknown to the relay).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, Notify};

use crate::crypto;
use crate::framed::{recv_msg, send_msg, wrap_incoming, FramedStream, Msg};

struct Room {
    first: Option<Box<dyn FramedStream>>,
    second: Option<Box<dyn FramedStream>>,
    opened: Instant,
    notify: Arc<Notify>,
}

pub struct Relay {
    rooms: Arc<Mutex<HashMap<String, Room>>>,
    relay_key: [u8; 32],
    ttl: Duration,
}

impl Relay {
    pub fn new(password: &str, ttl: Duration) -> Self {
        Self {
            rooms: Arc::new(Mutex::new(HashMap::new())),
            relay_key: crypto::derive_key(password.as_bytes()),
            ttl,
        }
    }

    /// Bind `listen` and serve forever. Supports both raw TCP and WebSocket
    /// connections on the same port (see [`crate::framed::wrap_incoming`]).
    pub async fn run(&self, listen: &str) -> anyhow::Result<()> {
        let listener = TcpListener::bind(listen).await?;
        eprintln!("RelayDrop relay listening on {listen}");
        self.spawn_cleanup();
        loop {
            let (stream, _addr) = listener.accept().await?;
            let rooms = self.rooms.clone();
            let relay_key = self.relay_key;
            let ttl = self.ttl;
            tokio::spawn(async move {
                if let Err(e) = handle_conn(stream, rooms, relay_key, ttl).await {
                    eprintln!("relay connection error: {e:#}");
                }
            });
        }
    }

    /// Background task: evict rooms that were never paired past their TTL.
    fn spawn_cleanup(&self) {
        let rooms = self.rooms.clone();
        let ttl = self.ttl;
        tokio::spawn(async move {
            loop {
                let interval = if ttl / 2 < Duration::from_secs(5) {
                    Duration::from_secs(5)
                } else {
                    ttl / 2
                };
                tokio::time::sleep(interval).await;
                let now = Instant::now();
                let mut g = rooms.lock().await;
                let stale: Vec<String> = g
                    .iter()
                    .filter(|(_, r)| r.second.is_none() && now.duration_since(r.opened) > ttl)
                    .map(|(k, _)| k.clone())
                    .collect();
                for k in stale {
                    if let Some(r) = g.remove(&k) {
                        drop(r); // closes the waiting peer's connection
                    }
                }
            }
        });
    }
}

async fn handle_conn(
    stream: TcpStream,
    rooms: Arc<Mutex<HashMap<String, Room>>>,
    relay_key: [u8; 32],
    ttl: Duration,
) -> anyhow::Result<()> {
    let mut conn = wrap_incoming(stream).await?;
    let peer = conn.peer_addr();

    // --- handshake: Password then RoomJoin ---
    match recv_msg(&mut *conn, &relay_key).await {
        Ok(Some(Msg::Password { .. })) => {}
        Ok(_) => {
            eprintln!("[{peer}] missing Password handshake, dropping");
            return Ok(());
        }
        Err(e) => {
            eprintln!("[{peer}] decrypt failed (likely --password mismatch with relay): {e:#}");
            return Ok(());
        }
    }
    let room = match recv_msg(&mut *conn, &relay_key).await? {
        Some(Msg::RoomJoin { room }) => room,
        _ => {
            eprintln!("[{peer}] expected RoomJoin, dropping");
            return Ok(());
        }
    };

    let mut g = rooms.lock().await;
    if g.contains_key(&room) {
        // Second (or third) peer.
        let entry = g.get_mut(&room).unwrap();
        if entry.second.is_some() {
            drop(g);
            send_msg(&mut *conn, &relay_key, &Msg::RoomFull).await?;
            eprintln!("[{peer}] room '{room}' full");
            return Ok(());
        }
        send_msg(&mut *conn, &relay_key, &Msg::Ready).await?;
        entry.second = Some(conn);
        let notify = entry.notify.clone();
        drop(g);
        notify.notify_one();

        // Take both streams and begin piping.
        let mut g2 = rooms.lock().await;
        let entry = g2.remove(&room).unwrap();
        drop(g2);
        let first = entry.first.expect("first peer present");
        let second = entry.second.expect("second peer present");
        eprintln!("[{peer}] room '{room}' paired, piping");
        pipe(first, second).await;
    } else {
        // First peer.
        send_msg(&mut *conn, &relay_key, &Msg::Ready).await?;
        let notify = Arc::new(Notify::new());
        g.insert(
            room.clone(),
            Room {
                first: Some(conn),
                second: None,
                opened: Instant::now(),
                notify: notify.clone(),
            },
        );
        drop(g);
        eprintln!("[{peer}] waiting in room '{room}'");
        match tokio::time::timeout(ttl, notify.notified()).await {
            Ok(()) => {
                // The second peer took over and is piping; nothing left to do.
            }
            Err(_) => {
                let mut g2 = rooms.lock().await;
                g2.remove(&room);
                eprintln!("[{peer}] room '{room}' timed out");
            }
        }
    }
    Ok(())
}

/// Full-duplex pipe between two streams. Forwards raw frames in both directions
/// until either side closes.
async fn pipe(mut a: Box<dyn FramedStream>, mut b: Box<dyn FramedStream>) {
    loop {
        tokio::select! {
            ra = a.recv_bytes() => {
                match ra {
                    Ok(Some(d)) => { if b.send_bytes(&d).await.is_err() { break; } }
                    _ => break,
                }
            }
            rb = b.recv_bytes() => {
                match rb {
                    Ok(Some(d)) => { if a.send_bytes(&d).await.is_err() { break; } }
                    _ => break,
                }
            }
        }
    }
}
