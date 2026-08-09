//! Portable logical workspace paths, independent of host filesystems.

use crate::error::LogicalPathError;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{fmt, str::FromStr};

#[derive(Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Debug)]
pub struct LogicalPath(String);

impl LogicalPath {
    /// Parses a portable, relative logical project path without host normalization.
    ///
    /// # Errors
    /// Returns [`LogicalPathError`] for an empty, oversized, absolute, malformed, or forbidden path.
    pub fn parse(value: &str) -> Result<Self, LogicalPathError> {
        if value.is_empty() {
            return Err(LogicalPathError::Empty);
        }
        if value.len() > 1024 {
            return Err(LogicalPathError::TooLong);
        }
        if value.starts_with('/') {
            return Err(LogicalPathError::Absolute);
        }
        if value.ends_with('/') {
            return Err(LogicalPathError::TrailingSlash);
        }
        if value.chars().any(|character| {
            character == '\\'
                || character == '\0'
                || character == ':'
                || character == '\u{7f}'
                || character.is_ascii_control()
        }) {
            return Err(LogicalPathError::ForbiddenCharacter);
        }
        for segment in value.split('/') {
            if segment.is_empty() {
                return Err(LogicalPathError::EmptySegment);
            }
            if segment == "." || segment == ".." {
                return Err(LogicalPathError::DotSegment);
            }
            if segment.len() > 255 {
                return Err(LogicalPathError::SegmentTooLong);
            }
        }
        Ok(Self(value.to_owned()))
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
    #[must_use]
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name();
        let (_, extension) = name.rsplit_once('.')?;
        (!extension.is_empty()).then_some(extension)
    }
}
impl fmt::Display for LogicalPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl FromStr for LogicalPath {
    type Err = LogicalPathError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}
impl Serialize for LogicalPath {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for LogicalPath {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(de::Error::custom)
    }
}
