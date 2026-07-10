//! Sender / receiver clients.
//!
//! Both roles connect to the relay, authenticate with the relay password,
//! join the room named by the shared `code`, then exchange file data. File
//! chunks are sealed with `file_key = HKDF(code)`, so the relay only sees
//! opaque ciphertext (p2p-file-transfer spec).
//!
//! Transfers are manifest-based (see design.md D1): the sender first sends a
//! `Manifest` listing every file (relative path, size, per-file SHA-256), then
//! streams each file's chunks followed by `FileEnd`, and finally `Done`.

use std::path::{Component, Path, PathBuf};

use base64::Engine as _;
use rand::Rng;
use sha2::Digest;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

use crate::crypto;
use crate::framed::{connect_client, recv_msg, send_msg, FileEntry, FramedStream, Msg};

const CHUNK: usize = 64 * 1024;

/// Connect + handshake, returning a ready `FramedStream` joined to `room`.
async fn connect(
    relay_url: &str,
    password: &str,
    room: &str,
) -> anyhow::Result<Box<dyn FramedStream>> {
    let relay_key = crypto::derive_key(password.as_bytes());
    let mut stream = connect_client(relay_url).await?;

    if let Err(e) = send_msg(
        &mut *stream,
        &relay_key,
        &Msg::Password {
            enc: crypto::encrypt(&relay_key, b"auth"),
        },
    )
    .await
    {
        anyhow::bail!(
            "failed to send auth to relay (connection reset? check relay address is reachable and --password matches the relay): {e:#}"
        );
    }
    if let Err(e) = send_msg(
        &mut *stream,
        &relay_key,
        &Msg::RoomJoin {
            room: room.to_string(),
        },
    )
    .await
    {
        anyhow::bail!("failed to send room join to relay: {e:#}");
    }

    match recv_msg(&mut *stream, &relay_key).await {
        Ok(Some(Msg::Ready)) => {}
        Ok(Some(Msg::RoomFull)) => anyhow::bail!("relay room is full"),
        Ok(Some(Msg::Error { msg })) => anyhow::bail!("relay error: {msg}"),
        Ok(other) => anyhow::bail!("unexpected relay response: {other:?}"),
        Err(e) => anyhow::bail!(
            "handshake with relay failed (likely --password mismatch with relay, or relay unreachable): {e:#}"
        ),
    }
    Ok(stream)
}

/// A random, URL-safe shared secret / relay password.
fn random_secret() -> String {
    let mut b = [0u8; 18];
    rand::rng().fill_bytes(&mut b);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// Public sender entry. The `code` (transfer secret) is generated randomly
/// when omitted; the relay `password` is the operator's fixed constant (passed
/// via `--password`, default empty) and is embedded in the printed command so
/// the receiver can authenticate to the already-running relay. `paths` is the
/// list of files/folders (or a mix) to transfer in a single session.
pub async fn send(
    relay_url: &str,
    password: &str,
    code: Option<&str>,
    paths: &[String],
) -> anyhow::Result<()> {
    let code = code.map(String::from).unwrap_or_else(random_secret);
    let pw_arg = if password.is_empty() {
        String::new()
    } else {
        format!(" --password {password}")
    };

    if password.is_empty() {
        eprintln!(
            "note: --password was not given. If the relay was started with --password, the sender AND receiver must use the SAME password;\n      add --password <same password> to the receiver command below, otherwise the relay will reject the connection (decrypt failed)."
        );
    } else {
        eprintln!("note: the receiver command below already embeds --password; the receiver can run it as-is (it must match the relay).");
    }

    println!(
        "On the other computer run:\n  relaydrop receive --relay {relay_url} --code {code}{pw_arg} --out ."
    );

    let mut stream = connect(relay_url, password, &code).await?;
    transfer_send(&mut *stream, &code, password, paths).await?;
    Ok(())
}

/// A manifest entry paired with the absolute source path on the sender's
/// machine. Only `entry` is sent over the wire; `source` is used locally to
/// re-open the file when streaming chunks.
struct ManifestItem {
    entry: FileEntry,
    source: PathBuf,
}

/// Build a flat manifest for all `inputs` (files, folders, or a mix).
/// Each entry's `relpath` is relative to the receiver's `--out` and preserves
/// the source's top-level name. Returns `(FileEntry, source_path)` pairs; the
/// source path is kept locally only for streaming and is never sent.
async fn build_manifest_for_paths(inputs: &[PathBuf]) -> anyhow::Result<Vec<ManifestItem>> {
    let mut items: Vec<ManifestItem> = Vec::new();
    for input in inputs {
        let meta = tokio::fs::metadata(input).await?;
        if meta.is_dir() {
            let base_name = input
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "dir".to_string());
            collect_dir(input, Path::new(&base_name), &mut items).await?;
        } else if meta.is_file() {
            let relpath = input
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "file".to_string());
            let sha = sha256_of_file(input).await?;
            items.push(ManifestItem {
                entry: FileEntry {
                    relpath,
                    size: meta.len(),
                    sha256: sha,
                },
                source: input.to_path_buf(),
            });
        } else {
            anyhow::bail!(
                "path is neither a regular file nor a directory: {}",
                input.display()
            );
        }
    }

    items.sort_by(|a, b| a.entry.relpath.cmp(&b.entry.relpath));

    // Reject duplicate top-level names (e.g. two different parents each with
    // `a.txt`, or two directories both named `data`) before sending anything.
    for w in items.windows(2) {
        if w[0].entry.relpath == w[1].entry.relpath {
            anyhow::bail!("duplicate entry name: {}", w[0].entry.relpath);
        }
    }

    if items.is_empty() {
        anyhow::bail!("no files to send (all inputs empty or skipped)");
    }
    Ok(items)
}

/// Recursively collect regular files under `dir`. `prefix` holds the path
/// components accumulated from the source's top-level name down to (but not
/// including) `dir`, so each `relpath` preserves the source tree exactly and
/// is rooted at the original top-level name on the receiver side.
async fn collect_dir(
    dir: &Path,
    prefix: &Path,
    out: &mut Vec<ManifestItem>,
) -> anyhow::Result<()> {
    let mut rd = tokio::fs::read_dir(dir).await?;
    while let Some(e) = rd.next_entry().await? {
        let p = e.path();
        let ft = e.file_type().await?;
        if ft.is_symlink() {
            eprintln!("skip symlink: {}", p.display());
            continue;
        }
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if ft.is_dir() {
            let new_prefix = prefix.join(&name);
            Box::pin(collect_dir(&p, &new_prefix, out)).await?;
        } else if ft.is_file() {
            let relpath = prefix.join(&name).to_string_lossy().replace('\\', "/");
            let m = tokio::fs::metadata(&p).await?;
            let sha = sha256_of_file(&p).await?;
            out.push(ManifestItem {
                entry: FileEntry {
                    relpath,
                    size: m.len(),
                    sha256: sha,
                },
                source: p,
            });
        } else {
            eprintln!("skip special file: {}", p.display());
        }
    }
    Ok(())
}

/// Streaming SHA-256 of a single file.
async fn sha256_of_file(path: &Path) -> anyhow::Result<String> {
    let mut f = tokio::fs::File::open(path).await?;
    let mut h = sha2::Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = f.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

/// Sender side: build the flat manifest for `paths`, send it, then stream each
/// file's chunks + FileEnd, then Done.
async fn transfer_send(
    stream: &mut dyn FramedStream,
    code: &str,
    password: &str,
    paths: &[String],
) -> anyhow::Result<()> {
    let relay_key = crypto::derive_key(password.as_bytes());
    let file_key = crypto::derive_key(code.as_bytes());

    let inputs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let items = build_manifest_for_paths(&inputs).await?;
    let entries: Vec<FileEntry> = items.iter().map(|i| i.entry.clone()).collect();
    send_msg(stream, &relay_key, &Msg::Manifest { entries }).await?;

    let mut total: u64 = 0;
    for item in &items {
        let mut f = tokio::fs::File::open(&item.source).await?;
        let mut sent: u64 = 0;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let n = f.read(&mut buf).await?;
            if n == 0 {
                break;
            }
            let sealed = crypto::encrypt(&file_key, &buf[..n]);
            send_msg(stream, &relay_key, &Msg::DataChunk(sealed)).await?;
            sent += n as u64;
            total += n as u64;
            eprint!(
                "\r\x1b[Ksent {sent}/{size} ({rel})",
                size = item.entry.size,
                rel = item.entry.relpath
            );
        }
        send_msg(stream, &relay_key, &Msg::FileEnd).await?;
    }
    send_msg(stream, &relay_key, &Msg::Done).await?;
    eprintln!();
    println!(
        "transfer complete: {} file(s), {total} bytes sent",
        items.len()
    );
    Ok(())
}

/// Public receiver entry.
pub async fn receive(
    relay_url: &str,
    password: &str,
    code: &str,
    out_dir: &str,
) -> anyhow::Result<()> {
    if password.is_empty() {
        eprintln!(
            "note: --password was not given. If the relay requires a password, add --password <same as relay/sender>."
        );
    }
    let relay_key = crypto::derive_key(password.as_bytes());
    let file_key = crypto::derive_key(code.as_bytes());
    let mut stream = connect(relay_url, password, code).await?;

    let entries = match recv_msg(&mut *stream, &relay_key).await? {
        Some(Msg::Manifest { entries }) => entries,
        Some(Msg::Error { msg }) => anyhow::bail!("sender error: {msg}"),
        other => anyhow::bail!("expected Manifest, got {other:?}"),
    };

    let base = Path::new(out_dir);

    let mut total: u64 = 0;
    for entry in &entries {
        let rel = safe_relpath(&entry.relpath);
        let out_path = base.join(&rel);
        if let Some(parent) = out_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut out = tokio::fs::File::create(&out_path).await?;

        let mut hasher = sha2::Sha256::new();
        let mut received: u64 = 0;
        loop {
            match recv_msg(&mut *stream, &relay_key).await? {
                Some(Msg::DataChunk(sealed)) => {
                    let plain = crypto::decrypt(&file_key, &sealed)?;
                    hasher.update(&plain);
                    out.write_all(&plain).await?;
                    received += plain.len() as u64;
                    total += plain.len() as u64;
                    eprint!(
                        "\r\x1b[Kreceived {received}/{size} ({rel})",
                        size = entry.size,
                        rel = entry.relpath
                    );
                }
                Some(Msg::FileEnd) => break,
                Some(Msg::Error { msg }) => anyhow::bail!("sender error: {msg}"),
                other => anyhow::bail!("unexpected message: {other:?}"),
            }
        }
        out.flush().await?;
        eprintln!();

        let actual = hex::encode(hasher.finalize());
        if actual != entry.sha256 {
            anyhow::bail!(
                "integrity check failed for {}: expected {}, got {actual}",
                entry.relpath,
                entry.sha256
            );
        }
    }

    match recv_msg(&mut *stream, &relay_key).await? {
        Some(Msg::Done) => {}
        other => anyhow::bail!("expected Done, got {other:?}"),
    }

    println!(
        "received {} file(s), {total} bytes, all sha256 verified",
        entries.len()
    );
    Ok(())
}

/// Keep only `Normal` path components, dropping `..` / root / cur / prefix to
/// prevent path traversal when reconstructing a received directory tree.
fn safe_relpath(rel: &str) -> PathBuf {
    let p = Path::new(rel);
    let mut out = PathBuf::new();
    for c in p.components() {
        if let Component::Normal(s) = c {
            out.push(s);
        }
    }
    if out.as_os_str().is_empty() {
        out.push("received.bin");
    }
    out
}
