use crate::{
    CitationUse, CommandUse, Comment, DependencyKind, DependencyRequest, DependencyTarget,
    DiagnosticCode, DiagnosticSeverity, DocumentClassRequest, EnvironmentOccurrence, FileAnalysis,
    LabelDefinition, MacroDefinition, MathKind, MathRegion, PackageRequest, ParserDiagnostic,
    ParserError, ParserLimits, ReferenceKind, ReferenceUse, Section, SectionLevel, SourceRange,
    TexDirective, TextEdit, queries,
};
use bytes::Bytes;
use core_types::LogicalPath;
use serde::{Deserialize, Serialize};
use tree_sitter::{InputEdit, Node, Parser, Point, Tree};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ParseMode {
    Full,
    Incremental,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParseStats {
    mode: ParseMode,
    generation: u64,
    source_bytes: u64,
    changed_ranges: Vec<SourceRange>,
}
impl ParseStats {
    #[must_use]
    pub const fn mode(&self) -> ParseMode {
        self.mode
    }
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }
    #[must_use]
    pub const fn source_bytes(&self) -> u64 {
        self.source_bytes
    }
    #[must_use]
    pub fn changed_ranges(&self) -> &[SourceRange] {
        &self.changed_ranges
    }
}

/// An owned synchronous incremental parsing session.
///
/// Analysis is best effort. Successful parsing is not a prerequisite for compilation, and real
/// TeX engines remain the semantic authority.
pub struct ParserSession {
    path: LogicalPath,
    source: Bytes,
    parser: Parser,
    tree: Tree,
    analysis: FileAnalysis,
    stats: ParseStats,
    limits: ParserLimits,
}
impl std::fmt::Debug for ParserSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ParserSession")
            .field("path", &self.path)
            .field("source_bytes", &self.source.len())
            .field("analysis", &self.analysis)
            .field("stats", &self.stats)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}
impl ParserSession {
    /// Creates a fully parsed session at generation one.
    ///
    /// # Errors
    /// Returns [`ParserError`] for grammar, query, parser, or offset infrastructure failures.
    pub fn new(
        path: LogicalPath,
        source: Bytes,
        limits: ParserLimits,
    ) -> Result<Self, ParserError> {
        let mut parser = initialized_parser()?;
        let tree = parser
            .parse(source.as_ref(), None)
            .ok_or(ParserError::ParseFailed)?;
        let analysis = analyze_file(&path, &source, &tree, limits)?;
        let source_bytes = u64::try_from(source.len()).map_err(|_| ParserError::OffsetOverflow)?;
        Ok(Self {
            path,
            source,
            parser,
            tree,
            analysis,
            stats: ParseStats {
                mode: ParseMode::Full,
                generation: 1,
                source_bytes,
                changed_ranges: Vec::new(),
            },
            limits,
        })
    }
    /// Creates a session using development extraction limits.
    ///
    /// # Errors
    /// Returns [`ParserError`] for grammar, query, parser, or offset infrastructure failures.
    pub fn with_default_limits(path: LogicalPath, source: Bytes) -> Result<Self, ParserError> {
        Self::new(path, source, ParserLimits::default())
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
    pub fn analysis(&self) -> &FileAnalysis {
        &self.analysis
    }
    #[must_use]
    pub fn stats(&self) -> &ParseStats {
        &self.stats
    }
    /// Atomically applies one byte edit using the edited previous Tree-sitter tree.
    ///
    /// # Errors
    /// Returns [`ParserError`] for invalid offsets, overflow, or parser infrastructure failure.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "the public mutation API owns an edit"
    )]
    pub fn apply_edit(&mut self, edit: TextEdit) -> Result<(), ParserError> {
        let source_len =
            u64::try_from(self.source.len()).map_err(|_| ParserError::OffsetOverflow)?;
        if edit.start_byte() > edit.old_end_byte() || edit.old_end_byte() > source_len {
            return Err(ParserError::InvalidEdit {
                start_byte: edit.start_byte(),
                old_end_byte: edit.old_end_byte(),
                source_len,
            });
        }
        let start_byte =
            usize::try_from(edit.start_byte()).map_err(|_| ParserError::OffsetOverflow)?;
        let old_end_byte =
            usize::try_from(edit.old_end_byte()).map_err(|_| ParserError::OffsetOverflow)?;
        let start_position = point_at(&self.source, start_byte);
        let old_end_position = point_at(&self.source, old_end_byte);
        let new_end_position = advance_point(start_position, edit.replacement());
        let new_end_byte = start_byte
            .checked_add(edit.replacement().len())
            .ok_or(ParserError::OffsetOverflow)?;
        let mut edited_tree = self.tree.clone();
        edited_tree.edit(&InputEdit {
            start_byte,
            old_end_byte,
            new_end_byte,
            start_position,
            old_end_position,
            new_end_position,
        });
        let mut candidate = Vec::with_capacity(
            self.source.len() - (old_end_byte - start_byte) + edit.replacement().len(),
        );
        candidate.extend_from_slice(&self.source[..start_byte]);
        candidate.extend_from_slice(edit.replacement());
        candidate.extend_from_slice(&self.source[old_end_byte..]);
        let candidate = Bytes::from(candidate);
        let new_tree = self
            .parser
            .parse(candidate.as_ref(), Some(&edited_tree))
            .ok_or(ParserError::ParseFailed)?;
        let analysis = analyze_file(&self.path, &candidate, &new_tree, self.limits)?;
        let changed_ranges = edited_tree
            .changed_ranges(&new_tree)
            .map(SourceRange::from_tree_sitter)
            .collect::<Result<Vec<_>, _>>()?;
        let generation = self
            .stats
            .generation
            .checked_add(1)
            .ok_or(ParserError::GenerationOverflow)?;
        let source_bytes =
            u64::try_from(candidate.len()).map_err(|_| ParserError::OffsetOverflow)?;
        self.source = candidate;
        self.tree = new_tree;
        self.analysis = analysis;
        self.stats = ParseStats {
            mode: ParseMode::Incremental,
            generation,
            source_bytes,
            changed_ranges,
        };
        Ok(())
    }

    /// Atomically replaces the complete source using full parse mode.
    ///
    /// # Errors
    /// Returns [`ParserError`] for overflow or parser infrastructure failure.
    pub fn replace_source(&mut self, source: Bytes) -> Result<(), ParserError> {
        let tree = self
            .parser
            .parse(source.as_ref(), None)
            .ok_or(ParserError::ParseFailed)?;
        let analysis = analyze_file(&self.path, &source, &tree, self.limits)?;
        let generation = self
            .stats
            .generation
            .checked_add(1)
            .ok_or(ParserError::GenerationOverflow)?;
        let source_bytes = u64::try_from(source.len()).map_err(|_| ParserError::OffsetOverflow)?;
        self.source = source;
        self.tree = tree;
        self.analysis = analysis;
        self.stats = ParseStats {
            mode: ParseMode::Full,
            generation,
            source_bytes,
            changed_ranges: Vec::new(),
        };
        Ok(())
    }
}

fn initialized_parser() -> Result<Parser, ParserError> {
    let language = tree_sitter_latex::language();
    queries::compile(&language)?;
    let mut parser = Parser::new();
    parser
        .set_language(&language)
        .map_err(|error| ParserError::GrammarInitialization {
            message: error.to_string(),
        })?;
    Ok(parser)
}
fn point_at(source: &[u8], offset: usize) -> Point {
    let mut point = Point::new(0, 0);
    for byte in &source[..offset] {
        if *byte == b'\n' {
            point.row += 1;
            point.column = 0;
        } else {
            point.column += 1;
        }
    }
    point
}
fn advance_point(mut point: Point, replacement: &[u8]) -> Point {
    for byte in replacement {
        if *byte == b'\n' {
            point.row += 1;
            point.column = 0;
        } else {
            point.column += 1;
        }
    }
    point
}

struct Extractor<'a> {
    source: &'a [u8],
    limits: ParserLimits,
    items: usize,
    text_bytes: usize,
    limited: bool,
    analysis: FileAnalysis,
}
impl<'a> Extractor<'a> {
    fn new(path: &LogicalPath, source: &'a [u8], limits: ParserLimits) -> Self {
        Self {
            source,
            limits,
            items: 0,
            text_bytes: 0,
            limited: false,
            analysis: FileAnalysis {
                path: path.clone(),
                document_classes: vec![],
                packages: vec![],
                sections: vec![],
                environments: vec![],
                labels: vec![],
                references: vec![],
                citations: vec![],
                dependencies: vec![],
                macros: vec![],
                commands: vec![],
                math_regions: vec![],
                comments: vec![],
                directives: vec![],
                diagnostics: vec![],
            },
        }
    }
    fn allow(&mut self, range: SourceRange) -> bool {
        if self.items < self.limits.max_semantic_items_per_file() {
            self.items += 1;
            true
        } else {
            if !self.limited {
                self.limited = true;
                self.analysis.diagnostics.push(ParserDiagnostic::new(
                    DiagnosticSeverity::Warning,
                    DiagnosticCode::ExtractionLimitReached,
                    "semantic item extraction limit reached",
                    Some(range),
                ));
            }
            false
        }
    }
    fn text(&mut self, node: Node<'_>, range: SourceRange) -> Option<String> {
        let bytes = &self.source[node.start_byte()..node.end_byte()];
        let bytes = if bytes.len() >= 2
            && ((bytes[0] == b'{' && bytes[bytes.len() - 1] == b'}')
                || (bytes[0] == b'[' && bytes[bytes.len() - 1] == b']'))
        {
            &bytes[1..bytes.len() - 1]
        } else {
            bytes
        };
        match std::str::from_utf8(bytes) {
            Ok(value)
                if self
                    .text_bytes
                    .checked_add(value.len())
                    .is_some_and(|total| total <= self.limits.max_extracted_text_bytes()) =>
            {
                self.text_bytes += value.len();
                Some(value.trim().to_owned())
            }
            Ok(_) => {
                if !self.limited {
                    self.limited = true;
                    self.analysis.diagnostics.push(ParserDiagnostic::new(
                        DiagnosticSeverity::Warning,
                        DiagnosticCode::ExtractionLimitReached,
                        "semantic text extraction limit reached",
                        Some(range),
                    ));
                }
                None
            }
            Err(_) => {
                self.analysis.diagnostics.push(ParserDiagnostic::new(
                    DiagnosticSeverity::Warning,
                    DiagnosticCode::InvalidUtf8SemanticText,
                    "semantic text is not valid UTF-8",
                    Some(range),
                ));
                None
            }
        }
    }
    fn command(&self, node: Node<'_>) -> Option<String> {
        node.child_by_field_name("command")
            .and_then(|command| {
                std::str::from_utf8(&self.source[command.start_byte()..command.end_byte()]).ok()
            })
            .map(|value| {
                value
                    .trim_start_matches('\\')
                    .trim_end_matches('*')
                    .to_owned()
            })
    }
}

fn analyze_file(
    path: &LogicalPath,
    source: &[u8],
    tree: &Tree,
    limits: ParserLimits,
) -> Result<FileAnalysis, ParserError> {
    let mut extractor = Extractor::new(path, source, limits);
    walk(tree.root_node(), &mut extractor)?;
    extractor.analysis.diagnostics.sort_by_key(|diagnostic| {
        (
            diagnostic.range().map_or(u64::MAX, SourceRange::start_byte),
            diagnostic.range().map_or(u64::MAX, SourceRange::end_byte),
            diagnostic.code(),
        )
    });
    Ok(extractor.analysis)
}

fn walk(node: Node<'_>, extractor: &mut Extractor<'_>) -> Result<(), ParserError> {
    let range = SourceRange::from_node(node)?;
    if node.is_error() {
        extractor.analysis.diagnostics.push(ParserDiagnostic::new(
            DiagnosticSeverity::Error,
            DiagnosticCode::SyntaxError,
            "LaTeX syntax error",
            Some(range),
        ));
    }
    if node.is_missing() {
        extractor.analysis.diagnostics.push(ParserDiagnostic::new(
            DiagnosticSeverity::Error,
            DiagnosticCode::MissingSyntax,
            "missing LaTeX syntax",
            Some(range),
        ));
    }
    extract_node(node, range, extractor);
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk(child, extractor)?;
    }
    Ok(())
}

fn extract_node(node: Node<'_>, range: SourceRange, x: &mut Extractor<'_>) {
    let kind = node.kind();
    let command = x.command(node);
    if let Some(name) = command.clone() {
        if x.allow(range) {
            x.analysis.commands.push(CommandUse::new(name, range));
        }
    }
    match kind {
        "class_include" => {
            if let Some(arg) = node.child_by_field_name("path") {
                if let Some(value) = x.text(arg, range) {
                    if x.allow(range) {
                        x.analysis
                            .document_classes
                            .push(DocumentClassRequest::new(value, range));
                    }
                }
            }
        }
        "package_include" => {
            if let Some(arg) = node.child_by_field_name("paths") {
                if let Some(value) = x.text(arg, range) {
                    for name in split_list(&value) {
                        if x.allow(range) {
                            x.analysis.packages.push(PackageRequest::new(name, range));
                        }
                    }
                }
            }
        }
        "part" | "chapter" | "section" | "subsection" | "subsubsection" | "paragraph"
        | "subparagraph" => extract_section(node, kind, range, x),
        "generic_environment" => extract_environment(node, range, x),
        "label_definition" => {
            if let Some(arg) = node.child_by_field_name("name") {
                if let Some(value) = x.text(arg, range).filter(|value| !value.is_empty()) {
                    if x.allow(range) {
                        x.analysis.labels.push(LabelDefinition::new(value, range));
                    }
                }
            }
        }
        "label_reference" => extract_reference(node, command, range, x),
        "citation" => {
            if let (Some(command), Some(arg)) = (command, node.child_by_field_name("keys")) {
                if let Some(value) = x.text(arg, range) {
                    let keys = split_list(&value);
                    if x.allow(range) {
                        x.analysis
                            .citations
                            .push(CitationUse::new(command, keys, range));
                    }
                }
            }
        }
        "latex_include" | "graphics_include" | "bibtex_include" | "biblatex_include" => {
            extract_dependency(node, kind, command.as_deref(), range, x);
        }
        "new_command_definition" | "old_command_definition" => {
            if let Some(arg) = node.child_by_field_name("declaration") {
                if let Some(value) = x.text(arg, range) {
                    let name = value
                        .trim_matches(|character| matches!(character, '{' | '}' | '\\'))
                        .to_owned();
                    if !name.is_empty() && x.allow(range) {
                        x.analysis.macros.push(MacroDefinition::new(name, range));
                    }
                }
            }
        }
        "generic_command" if command.is_none() => {
            if let Some(arg) = node.child_by_field_name("command") {
                if let Some(value) = x.text(arg, range) {
                    if x.allow(range) {
                        x.analysis.commands.push(CommandUse::new(
                            value.trim_start_matches('\\').to_owned(),
                            range,
                        ));
                    }
                }
            }
        }
        "inline_formula" => {
            if x.allow(range) {
                x.analysis
                    .math_regions
                    .push(MathRegion::new(MathKind::Inline, range));
            }
        }
        "displayed_equation" => {
            if x.allow(range) {
                x.analysis
                    .math_regions
                    .push(MathRegion::new(MathKind::Display, range));
            }
        }
        "comment" | "line_comment" => extract_comment(node, range, x),
        _ => {}
    }
}
fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}
fn extract_section(node: Node<'_>, kind: &str, range: SourceRange, x: &mut Extractor<'_>) {
    if let Some(arg) = node.child_by_field_name("text") {
        if let Some(title) = x.text(arg, range) {
            let level = match kind {
                "part" => SectionLevel::Part,
                "chapter" => SectionLevel::Chapter,
                "section" => SectionLevel::Section,
                "subsection" => SectionLevel::Subsection,
                "subsubsection" => SectionLevel::Subsubsection,
                "paragraph" => SectionLevel::Paragraph,
                _ => SectionLevel::Subparagraph,
            };
            let starred = x.source[node.start_byte()..node.end_byte()]
                .starts_with(format!("\\{kind}*").as_bytes());
            if x.allow(range) {
                x.analysis
                    .sections
                    .push(Section::new(level, title, starred, range));
            }
        }
    }
}
fn extract_environment(node: Node<'_>, range: SourceRange, x: &mut Extractor<'_>) {
    if let Some(begin) = node.child_by_field_name("begin") {
        if let Some(arg) = begin.named_child(0) {
            if let Some(name) = x.text(arg, range) {
                if x.allow(range) {
                    x.analysis
                        .environments
                        .push(EnvironmentOccurrence::new(name.clone(), range));
                }
                if matches!(
                    name.as_str(),
                    "equation" | "equation*" | "align" | "align*" | "gather" | "multline"
                ) && x.allow(range)
                {
                    x.analysis
                        .math_regions
                        .push(MathRegion::new(MathKind::Environment, range));
                }
            }
        }
    }
}
fn extract_reference(
    node: Node<'_>,
    command: Option<String>,
    range: SourceRange,
    x: &mut Extractor<'_>,
) {
    if let (Some(command), Some(arg)) = (command, node.child_by_field_name("names")) {
        if let Some(value) = x.text(arg, range) {
            let kind = match command.as_str() {
                "pageref" => ReferenceKind::PageRef,
                "eqref" => ReferenceKind::EqRef,
                "autoref" => ReferenceKind::AutoRef,
                "cref" => ReferenceKind::CRef,
                "Cref" => ReferenceKind::CapitalCRef,
                _ => ReferenceKind::Ref,
            };
            for key in split_list(&value) {
                if x.allow(range) {
                    x.analysis
                        .references
                        .push(ReferenceUse::new(kind, key, range));
                }
            }
        }
    }
}
fn extract_dependency(
    node: Node<'_>,
    syntax_kind: &str,
    command: Option<&str>,
    range: SourceRange,
    x: &mut Extractor<'_>,
) {
    let field = if syntax_kind == "bibtex_include" {
        "paths"
    } else {
        "path"
    };
    let arg = node
        .child_by_field_name(field)
        .or_else(|| node.named_child(node.named_child_count().saturating_sub(1)));
    if let Some(arg) = arg {
        if let Some(value) = x.text(arg, range) {
            let kind = match syntax_kind {
                "graphics_include" => DependencyKind::Graphics,
                "bibtex_include" | "biblatex_include" => DependencyKind::Bibliography,
                _ => match command {
                    Some("include") => DependencyKind::Include,
                    Some("subfile") => DependencyKind::Subfile,
                    _ => DependencyKind::Input,
                },
            };
            let values = if kind == DependencyKind::Bibliography {
                split_list(&value)
            } else {
                vec![value]
            };
            for raw in values {
                let dynamic = raw.contains('\\') || raw.contains('#');
                let target = if dynamic {
                    x.analysis.diagnostics.push(ParserDiagnostic::new(
                        DiagnosticSeverity::Information,
                        DiagnosticCode::DynamicDependency,
                        "dependency requires TeX expansion",
                        Some(range),
                    ));
                    DependencyTarget::Dynamic(raw)
                } else {
                    DependencyTarget::Static(raw)
                };
                if x.allow(range) {
                    x.analysis
                        .dependencies
                        .push(DependencyRequest::new(kind, target, range));
                }
            }
        }
    }
}
fn extract_comment(node: Node<'_>, range: SourceRange, x: &mut Extractor<'_>) {
    if let Some(text) = x.text(node, range) {
        if x.allow(range) {
            x.analysis.comments.push(Comment::new(text.clone(), range));
        }
        let body = text.trim_start_matches('%').trim();
        if let Some(rest) = body.strip_prefix("!TeX ") {
            if let Some((key, value)) = rest.split_once('=') {
                if x.allow(range) {
                    x.analysis.directives.push(TexDirective::new(
                        key.trim().to_owned(),
                        value.trim().to_owned(),
                        range,
                    ));
                }
            }
        }
    }
}
