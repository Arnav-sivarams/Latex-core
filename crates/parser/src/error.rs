use thiserror::Error;

/// Infrastructure failures. Syntax damage is represented by parser diagnostics instead.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum ParserError {
    #[error("failed to initialize the LaTeX grammar: {message}")]
    GrammarInitialization { message: String },
    #[error("failed to initialize a syntax query: {message}")]
    QueryInitialization { message: String },
    #[error("Tree-sitter unexpectedly returned no tree")]
    ParseFailed,
    #[error("invalid edit [{start_byte}, {old_end_byte}) for source length {source_len}")]
    InvalidEdit {
        start_byte: u64,
        old_end_byte: u64,
        source_len: u64,
    },
    #[error("a source offset cannot be represented safely")]
    OffsetOverflow,
    #[error("parser generation overflow")]
    GenerationOverflow,
    #[error("invalid project: {message}")]
    InvalidProject { message: String },
    #[error("internal parser invariant failed: {message}")]
    InternalInvariant { message: String },
}
