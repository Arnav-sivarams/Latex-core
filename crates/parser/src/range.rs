use crate::ParserError;
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Point, Range};

/// A zero-based source position with a byte-based column.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct SourcePoint {
    row: u32,
    column_bytes: u32,
}

impl SourcePoint {
    pub(crate) fn from_tree_sitter(value: Point) -> Result<Self, ParserError> {
        Ok(Self {
            row: u32::try_from(value.row).map_err(|_| ParserError::OffsetOverflow)?,
            column_bytes: u32::try_from(value.column).map_err(|_| ParserError::OffsetOverflow)?,
        })
    }
    #[must_use]
    pub const fn row(self) -> u32 {
        self.row
    }
    #[must_use]
    pub const fn column_bytes(self) -> u32 {
        self.column_bytes
    }
}

/// A validated, half-open source byte range `[start_byte, end_byte)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct SourceRange {
    start_byte: u64,
    end_byte: u64,
    start: SourcePoint,
    end: SourcePoint,
}

impl SourceRange {
    pub(crate) fn from_node(node: Node<'_>) -> Result<Self, ParserError> {
        Self::from_tree_sitter(node.range())
    }
    pub(crate) fn from_tree_sitter(value: Range) -> Result<Self, ParserError> {
        if value.start_byte > value.end_byte {
            return Err(ParserError::InternalInvariant {
                message: "reversed Tree-sitter range".into(),
            });
        }
        Ok(Self {
            start_byte: u64::try_from(value.start_byte).map_err(|_| ParserError::OffsetOverflow)?,
            end_byte: u64::try_from(value.end_byte).map_err(|_| ParserError::OffsetOverflow)?,
            start: SourcePoint::from_tree_sitter(value.start_point)?,
            end: SourcePoint::from_tree_sitter(value.end_point)?,
        })
    }
    #[must_use]
    pub const fn start_byte(self) -> u64 {
        self.start_byte
    }
    #[must_use]
    pub const fn end_byte(self) -> u64 {
        self.end_byte
    }
    #[must_use]
    pub const fn start(self) -> SourcePoint {
        self.start
    }
    #[must_use]
    pub const fn end(self) -> SourcePoint {
        self.end
    }
}
