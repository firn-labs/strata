//! Operator-owned encryption key (STORE-04).
//!
//! Blob encryption at rest uses XChaCha20-Poly1305 with a single key the
//! operating organization holds — bytes are encrypted *before* they reach a
//! storage provider, so no backend (in particular no external one) ever sees
//! plaintext the placement policy wants protected. Providers stay pure
//! byte stores and never know whether they hold ciphertext.
//!
//! Wire format of an encrypted blob: 24-byte random nonce, then the
//! ciphertext (which carries the AEAD tag). Whether a blob is encrypted is
//! recorded on the document's placement metadata, not guessed from bytes.
//!
//! Department-held keys (STORE-05), where administrators cannot decrypt,
//! are a separate later step — this key deliberately belongs to the
//! operator.

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

const NONCE_LEN: usize = 24;

/// The operator-owned key all at-rest encryption derives from.
pub struct OperatorKey {
    cipher: XChaCha20Poly1305,
}

impl OperatorKey {
    /// Parse a key from 64 hex characters (32 bytes), as configured via
    /// `STRATA_OPERATOR_KEY`.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.trim();
        let digits = hex.as_bytes();
        if digits.len() != 64 {
            return None;
        }
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            let hi = (digits[2 * i] as char).to_digit(16)?;
            let lo = (digits[2 * i + 1] as char).to_digit(16)?;
            *byte = (hi * 16 + lo) as u8;
        }
        Some(Self {
            cipher: XChaCha20Poly1305::new(Key::from_slice(&bytes)),
        })
    }

    /// A fresh random key. Blobs encrypted with it are unreadable once the
    /// key is dropped — fine for tests and development, and honest about it:
    /// production deployments configure a persistent key.
    pub fn generate() -> Self {
        Self {
            cipher: XChaCha20Poly1305::new(&XChaCha20Poly1305::generate_key(&mut OsRng)),
        }
    }

    /// Encrypt plaintext into the `nonce || ciphertext` blob format.
    pub fn encrypt(&self, plaintext: &[u8]) -> Vec<u8> {
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext)
            .expect("XChaCha20-Poly1305 encryption is infallible for in-memory buffers");
        let mut blob = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&ciphertext);
        blob
    }

    /// Decrypt a `nonce || ciphertext` blob. Fails when the blob is
    /// malformed, was encrypted under a different key, or was tampered with
    /// (the AEAD tag no longer verifies).
    pub fn decrypt(&self, blob: &[u8]) -> Option<Vec<u8>> {
        if blob.len() < NONCE_LEN {
            return None;
        }
        let (nonce, ciphertext) = blob.split_at(NONCE_LEN);
        self.cipher
            .decrypt(XNonce::from_slice(nonce), ciphertext)
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypt_then_decrypt_roundtrips() {
        let key = OperatorKey::generate();
        let blob = key.encrypt(b"strictly confidential bytes");
        assert_ne!(&blob[NONCE_LEN..], b"strictly confidential bytes");
        assert_eq!(key.decrypt(&blob).unwrap(), b"strictly confidential bytes");
    }

    #[test]
    fn decryption_fails_under_a_different_key() {
        let blob = OperatorKey::generate().encrypt(b"secret");
        assert_eq!(OperatorKey::generate().decrypt(&blob), None);
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let key = OperatorKey::generate();
        let mut blob = key.encrypt(b"secret");
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        assert_eq!(key.decrypt(&blob), None);
        assert_eq!(key.decrypt(b"too short"), None);
    }

    #[test]
    fn hex_keys_parse_strictly() {
        let hex = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
        let key = OperatorKey::from_hex(hex).unwrap();
        let again = OperatorKey::from_hex(hex).unwrap();
        assert_eq!(again.decrypt(&key.encrypt(b"data")).unwrap(), b"data");

        assert!(OperatorKey::from_hex("deadbeef").is_none(), "too short");
        assert!(
            OperatorKey::from_hex(&"zz".repeat(32)).is_none(),
            "not hex digits"
        );
    }
}
