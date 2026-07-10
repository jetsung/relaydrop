//! Cryptographic helpers: key derivation (HKDF-SHA256) and AES-256-GCM sealing.
//!
//! Two independent keys are used (see design.md D3):
//! - `relay_key`  = HKDF(relay_password)  -> protects the client<->relay channel
//! - `file_key`   = HKDF(room_code)       -> protects the peer<->peer file payload
//!
//! The relay never learns `file_key`, so it cannot read file contents.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use hkdf::Hkdf;
use rand::Rng;
use sha2::Sha256;

/// Derive a fixed 32-byte key from an arbitrary secret via HKDF-SHA256.
pub fn derive_key(secret: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(None, secret);
    let mut okm = [0u8; 32];
    // `expand` is infallible for a 32-byte output with this hash.
    hk.expand(b"relaydrop-v1", &mut okm)
        .expect("hkdf expand to 32 bytes");
    okm
}

/// Encrypt `plaintext` with `key`, prepending a fresh 12-byte random nonce.
/// Layout: `nonce(12) || ciphertext`.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    let mut nonce_bytes = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::try_from(nonce_bytes.as_slice()).expect("nonce is 12 bytes");
    let ct = cipher
        .encrypt(&nonce, plaintext)
        .expect("aes-gcm encrypt must succeed for valid key/nonce");
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    out
}

/// Decrypt a blob produced by [`encrypt`].
pub fn decrypt(key: &[u8; 32], blob: &[u8]) -> anyhow::Result<Vec<u8>> {
    if blob.len() < 12 {
        anyhow::bail!("ciphertext too short");
    }
    let (nonce_bytes, ct) = blob.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    let nonce = Nonce::try_from(nonce_bytes).expect("nonce is 12 bytes");
    let pt = cipher
        .decrypt(&nonce, ct)
        .map_err(|_| anyhow::anyhow!("decryption failed (wrong key or corrupted data)"))?;
    Ok(pt)
}
