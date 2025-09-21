use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::{Digest, Sha256};
use thiserror::Error;
use zeroize::Zeroizing;

type HmacSha256 = Hmac<Sha256>;

const KEY_LENGTH: usize = 32;
const NONCE_LENGTH: usize = 12;

/// Errors produced by the common-crypto helpers.
#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("invalid key length: expected {expected} bytes, got {actual}")]
    InvalidKeyLength { expected: usize, actual: usize },
    #[error("ciphertext missing nonce")]
    MissingNonce,
    #[error("encryption failure")]
    EncryptFailure,
    #[error("decryption failure")]
    DecryptFailure,
    #[error("base64 decode error: {0}")]
    Base64Decode(#[from] base64::DecodeError),
    #[error("invalid HMAC key length")]
    InvalidMacKey,
}

/// Wrapper around the tenant master key used to encrypt data encryption keys (DEKs).
#[derive(Clone)]
pub struct MasterKey(Zeroizing<[u8; KEY_LENGTH]>);

impl MasterKey {
    /// Construct a master key from a base64-encoded string.
    pub fn from_base64(value: &str) -> Result<Self, CryptoError> {
        let decoded = BASE64_STANDARD.decode(value.trim())?;
        Self::from_bytes(decoded)
    }

    /// Construct a master key from raw bytes.
    pub fn from_bytes<B>(bytes: B) -> Result<Self, CryptoError>
    where
        B: AsRef<[u8]>,
    {
        let slice = bytes.as_ref();
        if slice.len() != KEY_LENGTH {
            return Err(CryptoError::InvalidKeyLength {
                expected: KEY_LENGTH,
                actual: slice.len(),
            });
        }
        let mut array = [0u8; KEY_LENGTH];
        array.copy_from_slice(slice);
        Ok(Self(Zeroizing::new(array)))
    }

    /// Encrypt a tenant DEK for storage using AES-256-GCM.
    pub fn encrypt_tenant_dek(&self, dek: &[u8; KEY_LENGTH]) -> Result<Vec<u8>, CryptoError> {
        encrypt_with_key(&self.0, dek)
    }

    /// Decrypt the tenant DEK that was previously encrypted with this master key.
    pub fn decrypt_tenant_dek(&self, blob: &[u8]) -> Result<[u8; KEY_LENGTH], CryptoError> {
        let plaintext = decrypt_with_key(&self.0, blob)?;
        if plaintext.len() != KEY_LENGTH {
            return Err(CryptoError::InvalidKeyLength {
                expected: KEY_LENGTH,
                actual: plaintext.len(),
            });
        }
        let mut array = [0u8; KEY_LENGTH];
        array.copy_from_slice(&plaintext);
        Ok(array)
    }
}

impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MasterKey")
            .field("bytes", &"***redacted***")
            .finish()
    }
}

/// Generate a new random tenant DEK (32 bytes).
pub fn generate_dek() -> [u8; KEY_LENGTH] {
    let mut bytes = [0u8; KEY_LENGTH];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

/// Encrypt arbitrary plaintext with the supplied tenant DEK using AES-256-GCM.
pub fn encrypt_field(
    tenant_key: &[u8; KEY_LENGTH],
    plaintext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    encrypt_with_key(tenant_key, plaintext)
}

/// Decrypt previously encrypted ciphertext with the supplied tenant DEK.
pub fn decrypt_field(
    tenant_key: &[u8; KEY_LENGTH],
    ciphertext: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    decrypt_with_key(tenant_key, ciphertext)
}

/// Produce a deterministic HMAC-SHA256 hash for equality queries.
pub fn deterministic_hash(
    tenant_key: &[u8; KEY_LENGTH],
    value: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let mac_key = derive_hash_key(tenant_key);
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(&mac_key).map_err(|_| CryptoError::InvalidMacKey)?;
    mac.update(value);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn derive_hash_key(tenant_key: &[u8; KEY_LENGTH]) -> [u8; KEY_LENGTH] {
    let mut hasher = Sha256::new();
    hasher.update(tenant_key);
    hasher.update(b"novapos-hash-key");
    let digest = hasher.finalize();
    let mut out = [0u8; KEY_LENGTH];
    out.copy_from_slice(&digest);
    out
}

fn encrypt_with_key(key: &[u8; KEY_LENGTH], plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CryptoError::InvalidKeyLength {
        expected: KEY_LENGTH,
        actual: key.len(),
    })?;
    let mut nonce_bytes = [0u8; NONCE_LENGTH];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let mut ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| CryptoError::EncryptFailure)?;
    let mut output = Vec::with_capacity(NONCE_LENGTH + ciphertext.len());
    output.extend_from_slice(&nonce_bytes);
    output.append(&mut ciphertext);
    Ok(output)
}

fn decrypt_with_key(key: &[u8; KEY_LENGTH], ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
    if ciphertext.len() <= NONCE_LENGTH {
        return Err(CryptoError::MissingNonce);
    }
    let (nonce_bytes, encrypted) = ciphertext.split_at(NONCE_LENGTH);
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| CryptoError::InvalidKeyLength {
        expected: KEY_LENGTH,
        actual: key.len(),
    })?;
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), encrypted)
        .map_err(|_| CryptoError::DecryptFailure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_field_encryption() {
        let dek = generate_dek();
        let plaintext = b"sensitive-data";
        let ciphertext = encrypt_field(&dek, plaintext).expect("encrypt");
        assert_ne!(ciphertext, plaintext);
        let decrypted = decrypt_field(&dek, &ciphertext).expect("decrypt");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn envelope_encrypt_decrypt_dek() {
        let master = MasterKey::from_bytes([1u8; KEY_LENGTH]).expect("master");
        let dek = generate_dek();
        let blob = master.encrypt_tenant_dek(&dek).expect("encrypt dek");
        let recovered = master.decrypt_tenant_dek(&blob).expect("decrypt dek");
        assert_eq!(recovered, dek);
    }

    #[test]
    fn deterministic_hash_is_stable() {
        let dek = [7u8; KEY_LENGTH];
        let a = deterministic_hash(&dek, b"alice@example.com").expect("hash");
        let b = deterministic_hash(&dek, b"alice@example.com").expect("hash");
        let c = deterministic_hash(&dek, b"bob@example.com").expect("hash");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn base64_master_key_parsing() {
        let key = [9u8; KEY_LENGTH];
        let encoded = BASE64_STANDARD.encode(key);
        let parsed = MasterKey::from_base64(&encoded).expect("parse");
        let blob = parsed.encrypt_tenant_dek(&key).expect("encrypt");
        let recovered = parsed.decrypt_tenant_dek(&blob).expect("decrypt");
        assert_eq!(recovered, key);
    }
}
