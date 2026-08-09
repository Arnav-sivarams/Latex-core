use bytes::Bytes;

/// Replaces source bytes in `[start_byte, old_end_byte)` with arbitrary bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEdit {
    start_byte: u64,
    old_end_byte: u64,
    replacement: Bytes,
}
impl TextEdit {
    #[must_use]
    pub fn new(start_byte: u64, old_end_byte: u64, replacement: Bytes) -> Self {
        Self {
            start_byte,
            old_end_byte,
            replacement,
        }
    }
    #[must_use]
    pub const fn start_byte(&self) -> u64 {
        self.start_byte
    }
    #[must_use]
    pub const fn old_end_byte(&self) -> u64 {
        self.old_end_byte
    }
    #[must_use]
    pub fn replacement(&self) -> &Bytes {
        &self.replacement
    }
}
