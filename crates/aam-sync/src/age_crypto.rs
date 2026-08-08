//! `age`-based encryption for the two layers in
//! `docs/04-webdav-sync-security.md` §4.2:
//!
//! - **Layer 1** (`encrypt_with_passphrase`/`decrypt_with_passphrase`):
//!   protects `devices.json.age` with the user's master password.
//! - **Layer 2** (`encrypt_multi_recipient`/`decrypt_multi_recipient`):
//!   protects credential/index blobs, encrypted to every non-revoked
//!   device's X25519 public key.
//!
//! All functions here are pure (bytes in, bytes out) -- no I/O, no
//! `SyncBackend` dependency -- so they're straightforward to unit test with
//! round trips.

use age::secrecy::{ExposeSecret, SecretString};
use age::{Decryptor, Encryptor};
use std::error::Error;
use std::fmt;
use std::io::{Read, Write};
use std::str::FromStr;

#[derive(Debug)]
pub enum CryptoError {
    Encrypt(String),
    Decrypt(String),
    /// The ciphertext was a passphrase-encrypted file where a
    /// recipients-encrypted file was expected, or vice versa. Almost
    /// always means the caller reached for the wrong decrypt function.
    WrongDecryptorKind,
    InvalidRecipient(String),
    InvalidIdentity(String),
    Io(std::io::Error),
}

impl fmt::Display for CryptoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptoError::Encrypt(e) => write!(f, "age encryption failed: {e}"),
            CryptoError::Decrypt(e) => write!(f, "age decryption failed: {e}"),
            CryptoError::WrongDecryptorKind => write!(
                f,
                "age file's encryption kind (passphrase vs. recipients) didn't match what the caller expected"
            ),
            CryptoError::InvalidRecipient(e) => write!(f, "invalid age recipient key: {e}"),
            CryptoError::InvalidIdentity(e) => write!(f, "invalid age identity (private key): {e}"),
            CryptoError::Io(e) => write!(f, "I/O error during age (de)cryption: {e}"),
        }
    }
}

impl Error for CryptoError {}

impl From<std::io::Error> for CryptoError {
    fn from(e: std::io::Error) -> Self {
        CryptoError::Io(e)
    }
}

/// Layer 1: encrypts `plaintext` so it can only be decrypted with
/// `passphrase`.
pub fn encrypt_with_passphrase(plaintext: &[u8], passphrase: &str) -> Result<Vec<u8>, CryptoError> {
    let encryptor = Encryptor::with_user_passphrase(SecretString::new(passphrase.to_owned()));
    let mut out = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut out)
        .map_err(|e| CryptoError::Encrypt(e.to_string()))?;
    writer.write_all(plaintext)?;
    writer.finish().map_err(|e| CryptoError::Encrypt(e.to_string()))?;
    Ok(out)
}

/// Layer 1 inverse. Returns [`CryptoError::Decrypt`] on a wrong passphrase
/// (age deliberately doesn't distinguish "wrong passphrase" from "corrupt
/// file" in its error type, to avoid leaking oracle information).
pub fn decrypt_with_passphrase(ciphertext: &[u8], passphrase: &str) -> Result<Vec<u8>, CryptoError> {
    let decryptor = match Decryptor::new(ciphertext).map_err(|e| CryptoError::Decrypt(e.to_string()))? {
        Decryptor::Passphrase(d) => d,
        Decryptor::Recipients(_) => return Err(CryptoError::WrongDecryptorKind),
    };
    let mut reader = decryptor
        .decrypt(&SecretString::new(passphrase.to_owned()), None)
        .map_err(|e| CryptoError::Decrypt(e.to_string()))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

/// Layer 2: encrypts `plaintext` to every recipient in `recipients`
/// (bech32 `age1...` public key strings, `04.2`'s `age_public_key` field).
pub fn encrypt_multi_recipient(
    plaintext: &[u8],
    recipients: &[String],
) -> Result<Vec<u8>, CryptoError> {
    let parsed: Vec<Box<dyn age::Recipient + Send>> = recipients
        .iter()
        .map(|r| {
            age::x25519::Recipient::from_str(r)
                .map(|parsed| Box::new(parsed) as Box<dyn age::Recipient + Send>)
                .map_err(|e| CryptoError::InvalidRecipient(format!("{r}: {e}")))
        })
        .collect::<Result<_, _>>()?;

    let encryptor = Encryptor::with_recipients(parsed)
        .ok_or_else(|| CryptoError::Encrypt("no recipients provided".to_string()))?;
    let mut out = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut out)
        .map_err(|e| CryptoError::Encrypt(e.to_string()))?;
    writer.write_all(plaintext)?;
    writer.finish().map_err(|e| CryptoError::Encrypt(e.to_string()))?;
    Ok(out)
}

/// Layer 2 inverse: decrypts with a single device's private identity
/// string (bech32 `AGE-SECRET-KEY-1...`). Fails (rather than silently
/// producing garbage) if this identity isn't one of the blob's recipients
/// -- e.g. because the device has been revoked and the blob was
/// re-encrypted without it (`04.4`).
pub fn decrypt_multi_recipient(ciphertext: &[u8], identity: &str) -> Result<Vec<u8>, CryptoError> {
    let identity = age::x25519::Identity::from_str(identity)
        .map_err(|e| CryptoError::InvalidIdentity(e.to_string()))?;
    let decryptor = match Decryptor::new(ciphertext).map_err(|e| CryptoError::Decrypt(e.to_string()))? {
        Decryptor::Recipients(d) => d,
        Decryptor::Passphrase(_) => return Err(CryptoError::WrongDecryptorKind),
    };
    let identities: [&dyn age::Identity; 1] = [&identity];
    let mut reader = decryptor
        .decrypt(identities.into_iter())
        .map_err(|e| CryptoError::Decrypt(e.to_string()))?;
    let mut out = Vec::new();
    reader.read_to_end(&mut out)?;
    Ok(out)
}

/// Generates a new device identity for the "N devices, each with an X25519
/// keypair" model (`04.2`). Returns `(public_recipient_string,
/// private_identity_string)` -- the public string is safe to put in
/// `devices.json`; the private string must only ever be written to local,
/// OS-protected storage (`aam-vault::SecretStore`), never to WebDAV.
pub fn generate_device_keypair() -> (String, String) {
    let identity = age::x25519::Identity::generate();
    let public = identity.to_public().to_string();
    let private = identity.to_string().expose_secret().to_owned();
    (public, private)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passphrase_round_trip() {
        let plaintext = b"devices.json contents";
        let ciphertext = encrypt_with_passphrase(plaintext, "correct horse battery staple").unwrap();
        assert_ne!(ciphertext, plaintext);
        let decrypted =
            decrypt_with_passphrase(&ciphertext, "correct horse battery staple").unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn passphrase_wrong_password_fails() {
        let ciphertext = encrypt_with_passphrase(b"secret", "right-password").unwrap();
        let err = decrypt_with_passphrase(&ciphertext, "wrong-password").unwrap_err();
        assert!(matches!(err, CryptoError::Decrypt(_)));
    }

    #[test]
    fn multi_recipient_round_trip() {
        let (pub_a, priv_a) = generate_device_keypair();
        let (pub_b, priv_b) = generate_device_keypair();

        let ciphertext =
            encrypt_multi_recipient(b"provider config", &[pub_a.clone(), pub_b.clone()]).unwrap();

        assert_eq!(
            decrypt_multi_recipient(&ciphertext, &priv_a).unwrap(),
            b"provider config"
        );
        assert_eq!(
            decrypt_multi_recipient(&ciphertext, &priv_b).unwrap(),
            b"provider config"
        );
    }

    /// The core of `04.4`'s revocation guarantee: a device whose public key
    /// was never (or no longer) in the recipient list cannot decrypt.
    #[test]
    fn multi_recipient_revoked_device_cannot_decrypt() {
        let (pub_a, _priv_a) = generate_device_keypair();
        let (_pub_revoked, priv_revoked) = generate_device_keypair();

        // Blob re-encrypted with only device A as recipient (device
        // "revoked" was excluded, as `04.4` describes happening on the
        // next blob update after a revocation).
        let ciphertext = encrypt_multi_recipient(b"secret", &[pub_a]).unwrap();

        let err = decrypt_multi_recipient(&ciphertext, &priv_revoked).unwrap_err();
        assert!(matches!(err, CryptoError::Decrypt(_)));
    }

    #[test]
    fn generated_keypair_round_trips_through_string_parsing() {
        let (public, private) = generate_device_keypair();
        assert!(public.starts_with("age1"));
        assert!(private.to_uppercase().starts_with("AGE-SECRET-KEY-1"));

        let ciphertext = encrypt_multi_recipient(b"x", &[public]).unwrap();
        assert_eq!(decrypt_multi_recipient(&ciphertext, &private).unwrap(), b"x");
    }
}
