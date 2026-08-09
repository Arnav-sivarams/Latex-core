use crate::{ParserDiagnostic, ParserError, SourceRange};
use core_types::LogicalPath;
use serde::{Deserialize, Serialize};

macro_rules! name_range_type {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
        pub struct $name {
            name: String,
            range: SourceRange,
        }
        impl $name {
            pub(crate) fn new(name: String, range: SourceRange) -> Self {
                Self { name, range }
            }
            #[must_use]
            pub fn name(&self) -> &str {
                &self.name
            }
            #[must_use]
            pub const fn range(&self) -> SourceRange {
                self.range
            }
        }
    };
}
name_range_type!(DocumentClassRequest);
name_range_type!(PackageRequest);
name_range_type!(EnvironmentOccurrence);
name_range_type!(MacroDefinition);
name_range_type!(CommandUse);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum SectionLevel {
    Part,
    Chapter,
    Section,
    Subsection,
    Subsubsection,
    Paragraph,
    Subparagraph,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Section {
    level: SectionLevel,
    title: String,
    starred: bool,
    range: SourceRange,
}
impl Section {
    pub(crate) fn new(
        level: SectionLevel,
        title: String,
        starred: bool,
        range: SourceRange,
    ) -> Self {
        Self {
            level,
            title,
            starred,
            range,
        }
    }
    #[must_use]
    pub const fn level(&self) -> SectionLevel {
        self.level
    }
    #[must_use]
    pub fn title(&self) -> &str {
        &self.title
    }
    #[must_use]
    pub const fn starred(&self) -> bool {
        self.starred
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LabelDefinition {
    key: String,
    range: SourceRange,
}
impl LabelDefinition {
    pub(crate) fn new(key: String, range: SourceRange) -> Self {
        Self { key, range }
    }
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum ReferenceKind {
    Ref,
    PageRef,
    EqRef,
    AutoRef,
    CRef,
    CapitalCRef,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReferenceUse {
    kind: ReferenceKind,
    key: String,
    range: SourceRange,
}
impl ReferenceUse {
    pub(crate) fn new(kind: ReferenceKind, key: String, range: SourceRange) -> Self {
        Self { kind, key, range }
    }
    #[must_use]
    pub const fn kind(&self) -> ReferenceKind {
        self.kind
    }
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CitationUse {
    command: String,
    keys: Vec<String>,
    range: SourceRange,
}
impl CitationUse {
    pub(crate) fn new(command: String, keys: Vec<String>, range: SourceRange) -> Self {
        Self {
            command,
            keys,
            range,
        }
    }
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }
    #[must_use]
    pub fn keys(&self) -> &[String] {
        &self.keys
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum DependencyKind {
    Input,
    Include,
    Subfile,
    Graphics,
    Bibliography,
}
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum DependencyTarget {
    Static(String),
    Dynamic(String),
}
impl DependencyTarget {
    #[must_use]
    pub fn raw(&self) -> &str {
        match self {
            Self::Static(value) | Self::Dynamic(value) => value,
        }
    }
    #[must_use]
    pub const fn is_static(&self) -> bool {
        matches!(self, Self::Static(_))
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DependencyRequest {
    kind: DependencyKind,
    target: DependencyTarget,
    range: SourceRange,
}
impl DependencyRequest {
    pub(crate) fn new(kind: DependencyKind, target: DependencyTarget, range: SourceRange) -> Self {
        Self {
            kind,
            target,
            range,
        }
    }
    #[must_use]
    pub const fn kind(&self) -> DependencyKind {
        self.kind
    }
    #[must_use]
    pub fn target(&self) -> &DependencyTarget {
        &self.target
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum MathKind {
    Inline,
    Display,
    Environment,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MathRegion {
    kind: MathKind,
    range: SourceRange,
}
impl MathRegion {
    pub(crate) fn new(kind: MathKind, range: SourceRange) -> Self {
        Self { kind, range }
    }
    #[must_use]
    pub const fn kind(&self) -> MathKind {
        self.kind
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    text: String,
    range: SourceRange,
}
impl Comment {
    pub(crate) fn new(text: String, range: SourceRange) -> Self {
        Self { text, range }
    }
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TexDirective {
    key: String,
    value: String,
    range: SourceRange,
}
impl TexDirective {
    pub(crate) fn new(key: String, value: String, range: SourceRange) -> Self {
        Self { key, value, range }
    }
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }
    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}

/// Bounds static intelligence extraction without restricting future compilation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParserLimits {
    max_semantic_items_per_file: usize,
    max_extracted_text_bytes: usize,
}
impl ParserLimits {
    /// Constructs positive extraction limits.
    ///
    /// # Errors
    /// Returns [`ParserError`] when either limit is zero.
    pub fn new(
        max_semantic_items_per_file: usize,
        max_extracted_text_bytes: usize,
    ) -> Result<Self, ParserError> {
        if max_semantic_items_per_file == 0 || max_extracted_text_bytes == 0 {
            return Err(ParserError::InternalInvariant {
                message: "parser limits must be positive".into(),
            });
        }
        Ok(Self {
            max_semantic_items_per_file,
            max_extracted_text_bytes,
        })
    }
    #[must_use]
    pub const fn max_semantic_items_per_file(self) -> usize {
        self.max_semantic_items_per_file
    }
    #[must_use]
    pub const fn max_extracted_text_bytes(self) -> usize {
        self.max_extracted_text_bytes
    }
}
impl Default for ParserLimits {
    fn default() -> Self {
        Self {
            max_semantic_items_per_file: 100_000,
            max_extracted_text_bytes: 65_536,
        }
    }
}

/// Deterministically ordered best-effort analysis of one logical project file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileAnalysis {
    pub(crate) path: LogicalPath,
    pub(crate) document_classes: Vec<DocumentClassRequest>,
    pub(crate) packages: Vec<PackageRequest>,
    pub(crate) sections: Vec<Section>,
    pub(crate) environments: Vec<EnvironmentOccurrence>,
    pub(crate) labels: Vec<LabelDefinition>,
    pub(crate) references: Vec<ReferenceUse>,
    pub(crate) citations: Vec<CitationUse>,
    pub(crate) dependencies: Vec<DependencyRequest>,
    pub(crate) macros: Vec<MacroDefinition>,
    pub(crate) commands: Vec<CommandUse>,
    pub(crate) math_regions: Vec<MathRegion>,
    pub(crate) comments: Vec<Comment>,
    pub(crate) directives: Vec<TexDirective>,
    pub(crate) diagnostics: Vec<ParserDiagnostic>,
}
macro_rules! slice_access {
    ($method:ident, $field:ident, $item:ty) => {
        #[must_use]
        pub fn $method(&self) -> &[$item] {
            &self.$field
        }
    };
}
impl FileAnalysis {
    #[must_use]
    pub fn path(&self) -> &LogicalPath {
        &self.path
    }
    slice_access!(document_classes, document_classes, DocumentClassRequest);
    slice_access!(packages, packages, PackageRequest);
    slice_access!(sections, sections, Section);
    slice_access!(environments, environments, EnvironmentOccurrence);
    slice_access!(labels, labels, LabelDefinition);
    slice_access!(references, references, ReferenceUse);
    slice_access!(citations, citations, CitationUse);
    slice_access!(dependencies, dependencies, DependencyRequest);
    slice_access!(macros, macros, MacroDefinition);
    slice_access!(commands, commands, CommandUse);
    slice_access!(math_regions, math_regions, MathRegion);
    slice_access!(comments, comments, Comment);
    slice_access!(directives, directives, TexDirective);
    slice_access!(diagnostics, diagnostics, ParserDiagnostic);
}
