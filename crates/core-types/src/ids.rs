//! Strongly typed and validated identifiers.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use uuid::Uuid;

use crate::error::{IdentifierError, VersionOverflowError};

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }
            #[must_use]
            pub const fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl FromStr for $name {
            type Err = IdentifierError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value)
                    .map(Self)
                    .map_err(|error| IdentifierError::InvalidUuid(error.to_string()))
            }
        }
    };
}

uuid_id!(TenantId);
uuid_id!(UserId);
uuid_id!(WorkspaceId);
uuid_id!(JobId);
uuid_id!(ArtifactId);
uuid_id!(WorkerId);

macro_rules! validated_string_id {
    ($name:ident, $validator:ident) => {
        #[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
        pub struct $name(String);
        impl $name {
            /// Parses and validates the identifier.
            ///
            /// # Errors
            /// Returns [`IdentifierError`] when length, encoding, format, or characters violate
            /// this identifier's invariant.
            pub fn parse(value: &str) -> Result<Self, IdentifierError> {
                $validator(value)?;
                Ok(Self(value.to_owned()))
            }
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
        impl FromStr for $name {
            type Err = IdentifierError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::parse(value)
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).map_err(de::Error::custom)
            }
        }
    };
}

fn validate_extension(value: &str) -> Result<(), IdentifierError> {
    if !(3..=129).contains(&value.len()) {
        return Err(IdentifierError::InvalidLength {
            min: 3,
            max: 129,
            actual: value.len(),
        });
    }
    if !value.is_ascii() {
        return Err(IdentifierError::NonAscii);
    }
    let mut parts = value.split('.');
    let publisher = parts.next().unwrap_or_default();
    let name = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || publisher.is_empty()
        || name.is_empty()
        || publisher.len() > 64
        || name.len() > 64
    {
        return Err(IdentifierError::InvalidExtensionFormat);
    }
    for part in [publisher, name] {
        if !part
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(IdentifierError::InvalidCharacter);
        }
        if !part
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
            || !part
                .as_bytes()
                .last()
                .is_some_and(u8::is_ascii_alphanumeric)
        {
            return Err(IdentifierError::InvalidExtensionBoundary);
        }
    }
    Ok(())
}

fn validate_idempotency(value: &str) -> Result<(), IdentifierError> {
    validate_ascii_set(value, 1, 128, |byte| {
        byte.is_ascii_alphanumeric() || b"._-:".contains(&byte)
    })
}

fn validate_tex_environment(value: &str) -> Result<(), IdentifierError> {
    validate_ascii_set(value, 1, 128, |byte| {
        byte.is_ascii_alphanumeric() || b"._-:@+".contains(&byte)
    })
}

fn validate_latexmk_profile(value: &str) -> Result<(), IdentifierError> {
    validate_ascii_set(value, 1, 64, |byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"._-".contains(&byte)
    })
}

fn validate_ascii_set(
    value: &str,
    min: usize,
    max: usize,
    allowed: impl Fn(u8) -> bool,
) -> Result<(), IdentifierError> {
    if !(min..=max).contains(&value.len()) {
        return Err(IdentifierError::InvalidLength {
            min,
            max,
            actual: value.len(),
        });
    }
    if !value.is_ascii() {
        return Err(IdentifierError::NonAscii);
    }
    if !value.bytes().all(allowed) {
        return Err(IdentifierError::InvalidCharacter);
    }
    Ok(())
}

validated_string_id!(ExtensionId, validate_extension);
validated_string_id!(IdempotencyKey, validate_idempotency);
validated_string_id!(TexEnvironmentId, validate_tex_environment);
validated_string_id!(LatexmkProfileId, validate_latexmk_profile);

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceVersion(u64);

impl WorkspaceVersion {
    #[must_use]
    pub const fn initial() -> Self {
        Self(0)
    }
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
    /// Returns the next version.
    ///
    /// # Errors
    /// Returns [`VersionOverflowError`] at `u64::MAX` rather than wrapping.
    pub const fn checked_next(self) -> Result<Self, VersionOverflowError> {
        match self.0.checked_add(1) {
            Some(value) => Ok(Self(value)),
            None => Err(VersionOverflowError),
        }
    }
}
