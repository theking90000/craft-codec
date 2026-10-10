use crate::{Encryption, Result};

// Keep codec state inline so conversion requires no extra allocation.
#[allow(clippy::large_enum_variant)]
pub(crate) enum EncryptionState {
    None,
    #[cfg(feature = "aes-gcm")]
    Aes256Gcm(Aes256Gcm),
}

impl EncryptionState {
    pub(crate) fn new(parameters: &Encryption) -> Result<Self> {
        match parameters {
            Encryption::None => Ok(Self::None),
            #[cfg(feature = "aes-gcm")]
            Encryption::Aes256Gcm { key } => Ok(Self::Aes256Gcm(Aes256Gcm::new(key))),
            #[cfg(not(feature = "aes-gcm"))]
            Encryption::Aes256Gcm { .. } => Err(crate::Error::EncryptionUnavailable),
        }
    }

    pub(crate) fn seal(&self, _index: u64, _buffer: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::None => Ok(()),
            #[cfg(feature = "aes-gcm")]
            Self::Aes256Gcm(codec) => codec.seal(_index, _buffer),
        }
    }

    pub(crate) fn open(&self, _index: u64, _buffer: &mut Vec<u8>) -> Result<()> {
        match self {
            Self::None => Ok(()),
            #[cfg(feature = "aes-gcm")]
            Self::Aes256Gcm(codec) => codec.open(_index, _buffer),
        }
    }
}

/// AES-256-GCM with nonce `0u32 || index.to_be_bytes()`, empty AAD and a suffix tag.
///
/// Use one independent key per immutable object. Never encrypt distinct payloads
/// under the same key/index, even across encoder instances. Metadata must be
/// trusted. Key generation belongs to the caller; metadata persists the key.
#[cfg(feature = "aes-gcm")]
pub(crate) struct Aes256Gcm(aes_gcm::Aes256Gcm);

#[cfg(feature = "aes-gcm")]
impl Aes256Gcm {
    /// Initialize the expanded key once. The caller retains responsibility for
    /// its original key bytes. The underlying cipher enables its zeroize feature.
    pub(crate) fn new(key: &[u8; 32]) -> Self {
        use aes_gcm::KeyInit;
        Self(aes_gcm::Aes256Gcm::new(key.into()))
    }
}

#[cfg(feature = "aes-gcm")]
fn nonce(index: u64) -> aes_gcm::Nonce<aes_gcm::aead::consts::U12> {
    let [a, b, c, d, e, f, g, h] = index.to_be_bytes();
    [0, 0, 0, 0, a, b, c, d, e, f, g, h].into()
}

#[cfg(feature = "aes-gcm")]
impl Aes256Gcm {
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
        let tag: [u8; crate::TAG_LEN] = buffer
            .get(payload_len..)
            .ok_or(crate::Error::InvalidStoredLength)?
            .try_into()
            .map_err(|_| crate::Error::InvalidStoredLength)?;
        let payload = buffer
            .get_mut(..payload_len)
            .ok_or(crate::Error::InvalidStoredLength)?;
        self.0
            .decrypt_inout_detached(&nonce(index), b"", payload.into(), &tag.into())
            .map_err(|_| crate::Error::AuthenticationFailed)?;
        buffer.truncate(payload_len);
        Ok(())
    }
}
