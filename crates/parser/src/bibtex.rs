use crate::{DiagnosticCode, DiagnosticSeverity, ParserDiagnostic, ParserError, SourceRange};
use bytes::Bytes;
use core_types::LogicalPath;
use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser, Tree};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BibtexField {
    name: String,
    raw_value: String,
    range: SourceRange,
}
impl BibtexField {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BibtexEntry {
    entry_type: String,
    key: String,
    fields: Vec<BibtexField>,
    range: SourceRange,
}
impl BibtexEntry {
    #[must_use]
    pub fn entry_type(&self) -> &str {
        &self.entry_type
    }
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub fn fields(&self) -> &[BibtexField] {
        &self.fields
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BibtexStringDefinition {
    name: String,
    raw_value: String,
    range: SourceRange,
}
impl BibtexStringDefinition {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn raw_value(&self) -> &str {
        &self.raw_value
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BibtexAnalysis {
    entries: Vec<BibtexEntry>,
    string_definitions: Vec<BibtexStringDefinition>,
    diagnostics: Vec<ParserDiagnostic>,
}
impl BibtexAnalysis {
    #[must_use]
    pub fn entries(&self) -> &[BibtexEntry] {
        &self.entries
    }
    #[must_use]
    pub fn string_definitions(&self) -> &[BibtexStringDefinition] {
        &self.string_definitions
    }
    #[must_use]
    pub fn diagnostics(&self) -> &[ParserDiagnostic] {
        &self.diagnostics
    }
}

/// Best-effort static BibTeX parsing; BibTeX/Biber execution remains compilation authority.
pub struct BibtexSession {
    path: LogicalPath,
    source: Bytes,
    _parser: Parser,
    _tree: Tree,
    analysis: BibtexAnalysis,
}
impl std::fmt::Debug for BibtexSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BibtexSession")
            .field("path", &self.path)
            .field("source_bytes", &self.source.len())
            .field("analysis", &self.analysis)
            .finish_non_exhaustive()
    }
}
impl BibtexSession {
    /// Creates a fresh best-effort BibTeX parse.
    ///
    /// # Errors
    /// Returns [`ParserError`] when grammar initialization, parsing, or checked
    /// source-range conversion fails.
    pub fn new(path: LogicalPath, source: Bytes) -> Result<Self, ParserError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_bibtex::language())
            .map_err(|e| ParserError::GrammarInitialization {
                message: e.to_string(),
            })?;
        let tree = parser
            .parse(source.as_ref(), None)
            .ok_or(ParserError::ParseFailed)?;
        let analysis = analyze(&source, &tree)?;
        Ok(Self {
            path,
            source,
            _parser: parser,
            _tree: tree,
            analysis,
        })
    }
    #[must_use]
    pub fn path(&self) -> &LogicalPath {
        &self.path
    }
    #[must_use]
    pub fn source(&self) -> &Bytes {
        &self.source
    }
    #[must_use]
    pub fn analysis(&self) -> &BibtexAnalysis {
        &self.analysis
    }
}
fn analyze(source: &[u8], tree: &Tree) -> Result<BibtexAnalysis, ParserError> {
    let mut analysis = BibtexAnalysis::default();
    let root = tree.root_node();
    collect_syntax(root, &mut analysis.diagnostics)?;
    let mut cursor = root.walk();
    for node in root.named_children(&mut cursor) {
        match node.kind() {
            "entry" => extract_entry(node, source, &mut analysis)?,
            "string" => extract_string(node, source, &mut analysis)?,
            _ => {}
        }
    }
    analysis.entries.sort_by_key(|e| e.range.start_byte());
    analysis
        .string_definitions
        .sort_by_key(|e| e.range.start_byte());
    analysis.diagnostics.sort_by_key(|d| {
        (
            d.range().map_or(u64::MAX, SourceRange::start_byte),
            d.code(),
        )
    });
    Ok(analysis)
}
fn collect_syntax(
    node: Node<'_>,
    diagnostics: &mut Vec<ParserDiagnostic>,
) -> Result<(), ParserError> {
    if node.is_error() || node.is_missing() {
        diagnostics.push(ParserDiagnostic::new(
            DiagnosticSeverity::Error,
            DiagnosticCode::BibtexSyntaxError,
            "malformed BibTeX syntax",
            Some(SourceRange::from_node(node)?),
        ));
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_syntax(child, diagnostics)?;
    }
    Ok(())
}
fn text(
    node: Node<'_>,
    source: &[u8],
    diagnostics: &mut Vec<ParserDiagnostic>,
) -> Result<Option<String>, ParserError> {
    let range = SourceRange::from_node(node)?;
    if let Ok(value) = node.utf8_text(source) {
        Ok(Some(value.to_owned()))
    } else {
        diagnostics.push(ParserDiagnostic::new(
            DiagnosticSeverity::Error,
            DiagnosticCode::InvalidUtf8SemanticText,
            "BibTeX semantic text is not valid UTF-8",
            Some(range),
        ));
        Ok(None)
    }
}
fn extract_entry(
    node: Node<'_>,
    source: &[u8],
    analysis: &mut BibtexAnalysis,
) -> Result<(), ParserError> {
    let Some(ty) = node.child_by_field_name("ty") else {
        return Ok(());
    };
    let Some(key) = node.child_by_field_name("key") else {
        return Ok(());
    };
    let Some(entry_type) =
        text(ty, source, &mut analysis.diagnostics)?.map(|v| v.trim_start_matches('@').to_owned())
    else {
        return Ok(());
    };
    let Some(key) = text(key, source, &mut analysis.diagnostics)? else {
        return Ok(());
    };
    if key.is_empty() {
        return Ok(());
    }
    let mut fields = Vec::new();
    let mut cursor = node.walk();
    for child in node.children_by_field_name("field", &mut cursor) {
        let Some(name_node) = child.child_by_field_name("name") else {
            continue;
        };
        let Some(value_node) = child.child_by_field_name("value") else {
            continue;
        };
        let Some(name) = text(name_node, source, &mut analysis.diagnostics)? else {
            continue;
        };
        let Some(raw_value) = text(value_node, source, &mut analysis.diagnostics)? else {
            continue;
        };
        fields.push(BibtexField {
            name,
            raw_value,
            range: SourceRange::from_node(child)?,
        });
    }
    analysis.entries.push(BibtexEntry {
        entry_type,
        key,
        fields,
        range: SourceRange::from_node(node)?,
    });
    Ok(())
}
fn extract_string(
    node: Node<'_>,
    source: &[u8],
    analysis: &mut BibtexAnalysis,
) -> Result<(), ParserError> {
    let Some(name) = node.child_by_field_name("name") else {
        return Ok(());
    };
    let Some(value) = node.child_by_field_name("value") else {
        return Ok(());
    };
    let Some(name) = text(name, source, &mut analysis.diagnostics)? else {
        return Ok(());
    };
    let Some(raw_value) = text(value, source, &mut analysis.diagnostics)? else {
        return Ok(());
    };
    analysis.string_definitions.push(BibtexStringDefinition {
        name,
        raw_value,
        range: SourceRange::from_node(node)?,
    });
    Ok(())
}
