//! Fixed-size SHA-256 domain digests.

use crate::error::DigestParseError;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest, Sha256};
use std::{fmt, str::FromStr};

fn parse_digest(value: &str) -> Result<[u8; 32], DigestParseError> {
    if value.len() != 64 {
        return Err(DigestParseError::InvalidLength(value.len()));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DigestParseError::InvalidHex);
    }
    let mut bytes = [0; 32];
    hex::decode_to_slice(value, &mut bytes).map_err(|_| DigestParseError::InvalidHex)?;
    Ok(bytes)
}

macro_rules! digest_type {
    ($name:ident) => {
        #[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
        pub struct $name(pub(crate) [u8; 32]);
        impl $name {
            #[must_use]
            pub const fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
            #[must_use]
            pub fn to_hex(self) -> String {
                hex::encode(self.0)
            }
            pub(crate) fn hash_canonical(bytes: &[u8]) -> Self {
                Self(Sha256::digest(bytes).into())
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&hex::encode(self.0))
            }
        }
        impl FromStr for $name {
            type Err = DigestParseError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                parse_digest(value).map(Self)
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.to_hex())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(de::Error::custom)
            }
        }
    };
}

digest_type!(BlobHash);
digest_type!(SnapshotId);
digest_type!(CompileKey);

impl BlobHash {
    #[must_use]
    pub fn digest(bytes: &[u8]) -> Self {
        Self::hash_canonical(bytes)
    }
}
