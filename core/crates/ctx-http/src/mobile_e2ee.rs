use anyhow::{anyhow, Result};
use base64::Engine;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{Key, KeyInit, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand_core::RngCore;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey, StaticSecret};

const HKDF_INFO: &[u8] = b"ctx-mobile-e2ee-v1";

#[derive(Debug, Clone)]
pub struct E2eeKey([u8; 32]);

#[derive(Debug, Clone)]
pub struct Envelope {
    pub device_id: String,
    pub seq: i64,
    pub nonce_b64: String,
    pub ciphertext_b64: String,
}

pub fn generate_keypair() -> (String, String) {
    let secret = StaticSecret::random_from_rng(rand_core::OsRng);
    let public = PublicKey::from(&secret);
    let public_b64 = base64::engine::general_purpose::STANDARD.encode(public.as_bytes());
    let secret_b64 = base64::engine::general_purpose::STANDARD.encode(secret.to_bytes());
    (public_b64, secret_b64)
}

pub fn derive_key(
    device_id: &str,
    device_public_b64: &str,
    daemon_private_b64: &str,
) -> Result<E2eeKey> {
    let device_public = decode_key(device_public_b64)?;
    let daemon_private = decode_key(daemon_private_b64)?;
    let device_public = PublicKey::from(device_public);
    let daemon_private = StaticSecret::from(daemon_private);
    let shared = daemon_private.diffie_hellman(&device_public);

    let salt = Sha256::digest(device_id.as_bytes());
    let hk = Hkdf::<Sha256>::new(Some(&salt), shared.as_bytes());
    let mut out = [0u8; 32];
    hk.expand(HKDF_INFO, &mut out)
        .map_err(|_| anyhow!("hkdf expand failed"))?;
    Ok(E2eeKey(out))
}

pub fn encrypt(key: &E2eeKey, device_id: &str, seq: i64, plaintext: &[u8]) -> Result<Envelope> {
    let mut nonce = [0u8; 24];
    rand_core::OsRng.fill_bytes(&mut nonce);
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key.0));
    let aad = build_aad(device_id, seq);
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("encrypt failed"))?;

    Ok(Envelope {
        device_id: device_id.to_string(),
        seq,
        nonce_b64: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(nonce),
        ciphertext_b64: base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ciphertext),
    })
}

pub fn decrypt(
    key: &E2eeKey,
    device_id: &str,
    seq: i64,
    nonce_b64: &str,
    ciphertext_b64: &str,
) -> Result<Vec<u8>> {
    let nonce = decode_bytes(nonce_b64)?;
    let ciphertext = decode_bytes(ciphertext_b64)?;
    if nonce.len() != 24 {
        return Err(anyhow!("invalid nonce length"));
    }
    let cipher = XChaCha20Poly1305::new(Key::from_slice(&key.0));
    let aad = build_aad(device_id, seq);
    cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow!("decrypt failed"))
}

fn decode_key(value: &str) -> Result<[u8; 32]> {
    let bytes = decode_bytes(value)?;
    if bytes.len() != 32 {
        return Err(anyhow!("invalid key length"));
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn decode_bytes(value: &str) -> Result<Vec<u8>> {
    let mut normalized = value.trim().replace('-', "+").replace('_', "/");
    while !normalized.len().is_multiple_of(4) {
        normalized.push('=');
    }
    base64::engine::general_purpose::STANDARD
        .decode(normalized.as_bytes())
        .map_err(|_| anyhow!("invalid base64"))
}

fn build_aad(device_id: &str, seq: i64) -> Vec<u8> {
    format!("{device_id}:{seq}").into_bytes()
}
