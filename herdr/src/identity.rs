//! Local device identity and protected signing-key storage.
//!
//! The private Ed25519 key is encrypted with the operating system's protected
//! storage before it is written to disk. Only the public key, derived device
//! ID, and encrypted private-key blob are persisted. This module deliberately
//! does not log key material or expose the private key to callers.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

const IDENTITY_FILE_VERSION: u32 = 1;

pub(crate) struct DeviceIdentity {
    device_id: String,
    public_key: [u8; 32],
    signing_key: SigningKey,
}

#[derive(Deserialize, Serialize)]
struct PersistedIdentity {
    version: u32,
    device_id: String,
    public_key: String,
    protected_private_key: String,
}

#[derive(Debug)]
pub(crate) enum IdentityError {
    Io(std::io::Error),
    Serialization(serde_json::Error),
    Base64(base64::DecodeError),
    InvalidFile(&'static str),
    InvalidKey(&'static str),
    InvalidSignature,
    ProtectedStorage(String),
}

impl fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "identity storage I/O error: {error}"),
            Self::Serialization(error) => {
                write!(formatter, "identity serialization error: {error}")
            }
            Self::Base64(error) => write!(formatter, "identity encoding error: {error}"),
            Self::InvalidFile(message) => write!(formatter, "invalid identity file: {message}"),
            Self::InvalidKey(message) => write!(formatter, "invalid identity key: {message}"),
            Self::InvalidSignature => formatter.write_str("signature verification failed"),
            Self::ProtectedStorage(message) => {
                write!(formatter, "protected key storage unavailable: {message}")
            }
        }
    }
}

impl std::error::Error for IdentityError {}

impl From<std::io::Error> for IdentityError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for IdentityError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serialization(error)
    }
}

impl From<base64::DecodeError> for IdentityError {
    fn from(error: base64::DecodeError) -> Self {
        Self::Base64(error)
    }
}

impl DeviceIdentity {
    pub(crate) fn load_or_create(path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        let path = path.as_ref().to_path_buf();
        if path.exists() {
            return Self::load(path);
        }

        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key = signing_key.verifying_key().to_bytes();
        let device_id = device_id_for(&public_key);
        let persisted = PersistedIdentity {
            version: IDENTITY_FILE_VERSION,
            device_id: device_id.clone(),
            public_key: STANDARD.encode(public_key),
            protected_private_key: STANDARD.encode(protect_key(&signing_key.to_bytes())?),
        };
        write_new_file(&path, &persisted)?;

        Ok(Self {
            device_id,
            public_key,
            signing_key,
        })
    }

    fn load(path: PathBuf) -> Result<Self, IdentityError> {
        let bytes = fs::read(&path)?;
        let persisted: PersistedIdentity = serde_json::from_slice(&bytes)?;
        if persisted.version != IDENTITY_FILE_VERSION {
            return Err(IdentityError::InvalidFile(
                "unsupported identity file version",
            ));
        }

        let public_key_bytes = STANDARD.decode(persisted.public_key)?;
        let public_key: [u8; 32] = public_key_bytes
            .try_into()
            .map_err(|_| IdentityError::InvalidKey("public key must contain 32 bytes"))?;
        VerifyingKey::from_bytes(&public_key)
            .map_err(|_| IdentityError::InvalidKey("public key is not a valid Ed25519 key"))?;

        let protected_private_key = STANDARD.decode(persisted.protected_private_key)?;
        let private_key_bytes = unprotect_key(&protected_private_key)?;
        let private_key: [u8; 32] = private_key_bytes
            .try_into()
            .map_err(|_| IdentityError::InvalidKey("private key must contain 32 bytes"))?;
        let signing_key = SigningKey::from_bytes(&private_key);
        if signing_key.verifying_key().to_bytes() != public_key {
            return Err(IdentityError::InvalidFile(
                "public key does not match protected private key",
            ));
        }

        let derived_device_id = device_id_for(&public_key);
        if persisted.device_id != derived_device_id {
            return Err(IdentityError::InvalidFile(
                "device ID does not match public key",
            ));
        }

        Ok(Self {
            device_id: derived_device_id,
            public_key,
            signing_key,
        })
    }

    pub(crate) fn load_existing(path: impl AsRef<Path>) -> Result<Self, IdentityError> {
        Self::load(path.as_ref().to_path_buf())
    }

    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
    }

    pub(crate) fn public_key(&self) -> String {
        STANDARD.encode(self.public_key)
    }

    pub(crate) fn sign(&self, message: &[u8]) -> String {
        STANDARD.encode(self.signing_key.sign(message).to_bytes())
    }

    pub(crate) fn verify(
        public_key: &str,
        message: &[u8],
        signature: &str,
    ) -> Result<(), IdentityError> {
        let public_key_bytes: [u8; 32] = STANDARD
            .decode(public_key)?
            .try_into()
            .map_err(|_| IdentityError::InvalidKey("public key must contain 32 bytes"))?;
        let verifying_key = VerifyingKey::from_bytes(&public_key_bytes)
            .map_err(|_| IdentityError::InvalidKey("public key is not a valid Ed25519 key"))?;
        let signature_bytes: [u8; 64] = STANDARD
            .decode(signature)?
            .try_into()
            .map_err(|_| IdentityError::InvalidSignature)?;
        let signature = Signature::from_bytes(&signature_bytes);
        verifying_key
            .verify(message, &signature)
            .map_err(|_| IdentityError::InvalidSignature)
    }
}

fn device_id_for(public_key: &[u8; 32]) -> String {
    let mut digest = Sha256::new();
    digest.update(b"team-assistant/device-id/v1");
    digest.update(public_key);
    format!("device-{}", hex_encode(&digest.finalize()))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn write_new_file(path: &Path, persisted: &PersistedIdentity) -> Result<(), IdentityError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let serialized = serde_json::to_vec_pretty(persisted)?;
    let temp_path = path.with_extension(format!("identity-{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)?;
    file.write_all(&serialized)?;
    file.sync_all()?;
    drop(file);
    if let Err(error) = fs::rename(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(windows)]
fn protect_key(data: &[u8; 32]) -> Result<Vec<u8>, IdentityError> {
    use std::ptr;
    use std::slice;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let succeeded = unsafe {
        CryptProtectData(
            &input,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if succeeded == 0 || output.pbData.is_null() {
        return Err(IdentityError::ProtectedStorage(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    let protected =
        unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(protected)
}

#[cfg(not(windows))]
fn protect_key(_data: &[u8; 32]) -> Result<Vec<u8>, IdentityError> {
    Err(IdentityError::ProtectedStorage(
        "this platform has no implemented protected-key backend".into(),
    ))
}

#[cfg(windows)]
fn unprotect_key(data: &[u8]) -> Result<Vec<u8>, IdentityError> {
    use std::ptr;
    use std::slice;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: ptr::null_mut(),
    };
    let succeeded = unsafe {
        CryptUnprotectData(
            &input,
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    };
    if succeeded == 0 || output.pbData.is_null() {
        return Err(IdentityError::ProtectedStorage(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    let unprotected =
        unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(unprotected)
}

#[cfg(not(windows))]
fn unprotect_key(_data: &[u8]) -> Result<Vec<u8>, IdentityError> {
    Err(IdentityError::ProtectedStorage(
        "this platform has no implemented protected-key backend".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn identity_path(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "team-assistant-identity-{label}-{}-{nanos}.json",
            std::process::id()
        ))
    }

    #[test]
    fn identity_persists_and_verifies_signed_records() {
        let path = identity_path("persist");
        let first = DeviceIdentity::load_or_create(&path).expect("create identity");
        let public_key = first.public_key();
        let device_id = first.device_id().to_string();
        let signature = first.sign(b"signed record");
        DeviceIdentity::verify(&public_key, b"signed record", &signature)
            .expect("verify signature");
        assert!(DeviceIdentity::verify(&public_key, b"tampered record", &signature).is_err());
        drop(first);

        let second = DeviceIdentity::load_or_create(&path).expect("reload identity");
        assert_eq!(second.device_id(), device_id);
        assert_eq!(second.public_key(), public_key);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn identity_file_contains_no_plaintext_private_key() {
        let path = identity_path("protected");
        let identity = DeviceIdentity::load_or_create(&path).expect("create identity");
        let private_key = identity.signing_key.to_bytes();
        let file_bytes = fs::read(&path).expect("read identity file");
        assert!(!file_bytes
            .windows(private_key.len())
            .any(|window| window == private_key));
        let _ = fs::remove_file(path);
    }
}
