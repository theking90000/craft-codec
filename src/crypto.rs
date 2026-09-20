use crate::{Encryption, Identity, Result};

/// Sealed encryption implementation used for static dispatch.
/// Applications normally pass [`Identity`] or, with feature `aes-gcm`, `Aes256Gcm`.
pub trait EncryptionCodec: crate::sealed::Sealed {
    /// Metadata format implemented by this codec.
    const FORMAT: Encryption;
    /// Encrypt a payload and append its tag.
    #[doc(hidden)]
    fn seal(&self, index: u64, buffer: &mut Vec<u8>) -> Result<()>;
    /// Authenticate/decrypt a stored frame and remove its tag.
    #[doc(hidden)]
    fn open(&self, index: u64, buffer: &mut Vec<u8>) -> Result<()>;
}

impl EncryptionCodec for Identity {
    const FORMAT: Encryption = Encryption::None;
    fn seal(&self, _: u64, _: &mut Vec<u8>) -> Result<()> {
        Ok(())
    }
    fn open(&self, _: u64, _: &mut Vec<u8>) -> Result<()> {
        Ok(())
    }
}

/// AES-256-GCM with nonce `0u32 || index.to_be_bytes()`, empty AAD and a suffix tag.
///
/// Use one independent key per immutable object. Never encrypt distinct payloads
/// under the same key/index, even across encoder instances. Metadata must be
/// trusted. Key generation and persistence belong to the caller.
#[cfg(feature = "aes-gcm")]
pub struct Aes256Gcm(aes_gcm::Aes256Gcm);

#[cfg(feature = "aes-gcm")]
impl Aes256Gcm {
    /// Initialize the expanded key once. The caller retains responsibility for
    /// its original key bytes. The underlying cipher enables its zeroize feature.
    pub fn new(key: &[u8; 32]) -> Self {
        use aes_gcm::KeyInit;
        Self(aes_gcm::Aes256Gcm::new(key.into()))
    }
}

#[cfg(feature = "aes-gcm")]
impl crate::sealed::Sealed for Aes256Gcm {}

#[cfg(feature = "aes-gcm")]
fn nonce(index: u64) -> aes_gcm::Nonce<aes_gcm::aead::consts::U12> {
    let mut bytes = [0; 12];
    bytes[4..].copy_from_slice(&index.to_be_bytes());
    bytes.into()
}

#[cfg(feature = "aes-gcm")]
impl EncryptionCodec for Aes256Gcm {
    const FORMAT: Encryption = Encryption::Aes256Gcm;

    fn seal(&self, index: u64, buffer: &mut Vec<u8>) -> Result<()> {
        use aes_gcm::aead::AeadInOut;
        buffer
            .try_reserve(crate::TAG_LEN)
            .map_err(|_| crate::Error::Allocation)?;
        let tag = self
            .0
            .encrypt_inout_detached(&nonce(index), b"", buffer.as_mut_slice().into())
            .map_err(|_| crate::Error::EncryptionFailed)?;
        buffer.extend_from_slice(&tag);
        Ok(())
    }

    fn open(&self, index: u64, buffer: &mut Vec<u8>) -> Result<()> {
        use aes_gcm::aead::AeadInOut;
        let payload_len = buffer
            .len()
            .checked_sub(crate::TAG_LEN)
            .ok_or(crate::Error::InvalidStoredLength)?;
        let tag: [u8; crate::TAG_LEN] = buffer[payload_len..].try_into().unwrap();
        self.0
            .decrypt_inout_detached(
                &nonce(index),
                b"",
                (&mut buffer[..payload_len]).into(),
                &tag.into(),
            )
            .map_err(|_| crate::Error::AuthenticationFailed)?;
        buffer.truncate(payload_len);
        Ok(())
    }
}
