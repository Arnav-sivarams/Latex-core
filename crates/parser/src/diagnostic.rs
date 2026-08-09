use crate::SourceRange;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Information,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum DiagnosticCode {
    SyntaxError,
    MissingSyntax,
    InvalidUtf8SemanticText,
    ExtractionLimitReached,
    DynamicDependency,
    MissingProjectDependency,
    DependencyCycle,
    DuplicateLabel,
    UnresolvedReference,
}

/// A stable best-effort parsing or static-analysis diagnostic.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParserDiagnostic {
    severity: DiagnosticSeverity,
    code: DiagnosticCode,
    message: String,
    range: Option<SourceRange>,
}
impl ParserDiagnostic {
    pub(crate) fn new(
        severity: DiagnosticSeverity,
        code: DiagnosticCode,
        message: impl Into<String>,
        range: Option<SourceRange>,
    ) -> Self {
        Self {
            severity,
            code,
            message: message.into(),
            range,
        }
    }
    #[must_use]
    pub const fn severity(&self) -> DiagnosticSeverity {
        self.severity
    }
    #[must_use]
    pub const fn code(&self) -> DiagnosticCode {
        self.code
    }
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
    #[must_use]
    pub const fn range(&self) -> Option<SourceRange> {
        self.range
    }
}
