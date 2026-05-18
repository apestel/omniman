use aes_gcm::{aead::Aead, Aes256Gcm, Key, KeyInit, Nonce};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use getrandom::getrandom;
use secret_service::{EncryptionType, SecretService};
use std::collections::HashMap;

const ITEM_LABEL: &str = "Omniman Clipboard Encryption Key";
const ITEM_ATTR_KEY: &str = "org.adrien.omniman.clip-key";

/// Manages encryption/decryption of clipboard content.
pub struct ClipEncryption {
    key: Aes256Gcm,
}

impl ClipEncryption {
    /// Initialize encryption by fetching or generating a key in GNOME Keyring.
    /// Must be called from within a tokio runtime.
    pub async fn new() -> Result<Self> {
        let ss = SecretService::connect(EncryptionType::Dh)
            .await
            .context("connecting to Secret Service")?;

        let collection = ss
            .get_default_collection()
            .await
            .context("opening default Secret Service collection")?;

        if collection.is_locked().await.unwrap_or(false) {
            let _ = collection.unlock().await;
        }

        // Try to find existing key
        let mut attrs = HashMap::new();
        attrs.insert(ITEM_ATTR_KEY, "");
        let search_result = ss
            .search_items(attrs)
            .await
            .context("searching for key in Secret Service")?;

        let key = if let Some(item) = search_result.unlocked.first() {
            if let Ok(secret_bytes) = item.get_secret().await {
                let b64 = String::from_utf8_lossy(&secret_bytes);
                if let Ok(key_bytes) = STANDARD.decode(b64.as_ref()) {
                    if key_bytes.len() == 32 {
                        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
                        Some(Aes256Gcm::new(key))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else if let Some(locked_item) = search_result.locked.first() {
            // Try unlocking locked items
            let _ = locked_item.unlock().await;
            if let Ok(secret_bytes) = locked_item.get_secret().await {
                let b64 = String::from_utf8_lossy(&secret_bytes);
                if let Ok(key_bytes) = STANDARD.decode(b64.as_ref()) {
                    if key_bytes.len() == 32 {
                        let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
                        Some(Aes256Gcm::new(key))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // If no key found, generate and store one
        let key = match key {
            Some(k) => k,
            None => {
                let mut key_bytes = [0u8; 32];
                getrandom(&mut key_bytes)
                    .map_err(|e| anyhow::anyhow!("generating encryption key: {}", e))?;
                let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
                let b64 = STANDARD.encode(&key_bytes);
                let mut create_attrs = HashMap::new();
                create_attrs.insert(ITEM_ATTR_KEY, "1");
                let _ = collection
                    .create_item(
                        ITEM_LABEL,
                        create_attrs,
                        b64.as_bytes(),
                        true,
                        "text/plain",
                    )
                    .await;
                cipher
            }
        };

        Ok(Self { key })
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut nonce_buf = [0u8; 12];
        getrandom(&mut nonce_buf)
            .map_err(|e| anyhow::anyhow!("generating nonce: {}", e))?;
        let nonce = Nonce::from_slice(&nonce_buf);
        let ciphertext = self
            .key
            .encrypt(nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("encrypting: {}", e))?;
        let mut result = Vec::with_capacity(12 + ciphertext.len());
        result.extend_from_slice(&nonce_buf);
        result.extend_from_slice(&ciphertext);
        Ok(result)
    }

    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        if data.len() < 12 {
            return Ok(data.to_vec());
        }
        let (nonce_bytes, ct) = data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = self
            .key
            .decrypt(nonce, ct)
            .map_err(|e| anyhow::anyhow!("decrypting: {}", e))?;
        Ok(plaintext)
    }
}
