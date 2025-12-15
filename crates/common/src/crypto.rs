//! Cryptographic utilities for Polyglot-AI

use ring::digest::{Context, SHA256};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("Hash computation failed")]
    HashError,
    #[error("Base64 decode error: {0}")]
    Base64Error(#[from] base64::DecodeError),
}

pub fn sha256_hex(data: &[u8]) -> String {
    let mut context = Context::new(&SHA256);
    context.update(data);
    let digest = context.finish();
    hex_encode(digest.as_ref())
}

pub fn sha256_base64(data: &[u8]) -> String {
    let mut context = Context::new(&SHA256);
    context.update(data);
    let digest = context.finish();
    BASE64.encode(digest.as_ref())
}

pub fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn hex_decode(s: &str) -> Result<Vec<u8>, CryptoError> {
    if s.len() % 2 != 0 {
        return Err(CryptoError::HashError);
    }
    
    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(CryptoError::HashError);
    }

    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| CryptoError::HashError)
        })
        .collect()
}

pub fn random_token(len: usize) -> String {
    use ring::rand::{SecureRandom, SystemRandom};

    let rng = SystemRandom::new();
    let mut bytes = vec![0u8; len];
    rng.fill(&mut bytes).expect("Failed to generate random bytes");
    BASE64.encode(&bytes)
}

pub fn cert_fingerprint(cert_der: &[u8]) -> String {
    sha256_hex(cert_der)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hex() {
        let hash = sha256_hex(b"hello world");
        assert_eq!(
            hash,
            "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"
        );
    }

    #[test]
    fn test_hex_encode_decode() {
        let data = b"test data";
        let encoded = hex_encode(data);
        let decoded = hex_decode(&encoded).unwrap();
        assert_eq!(data.as_slice(), decoded.as_slice());
    }

    #[test]
    fn test_random_token() {
        let token1 = random_token(32);
        let token2 = random_token(32);
        assert_ne!(token1, token2);
        assert!(!token1.is_empty());
    }
}
