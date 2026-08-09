use crate::{
    BibtexAnalysis, BibtexSession, DependencyKind, DependencyTarget, DiagnosticCode,
    DiagnosticSeverity, FileAnalysis, ParserDiagnostic, ParserError, ParserLimits, ParserSession,
    SourceRange,
};
use bytes::Bytes;
use core_types::LogicalPath;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Immutable byte input for deterministic project analysis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectSource {
    main_file: LogicalPath,
    files: BTreeMap<LogicalPath, Bytes>,
}
impl ProjectSource {
    /// Validates that the file map is non-empty and contains the main file.
    ///
    /// # Errors
    /// Returns [`ParserError::InvalidProject`] when either invariant is violated.
    pub fn new(
        main_file: LogicalPath,
        files: BTreeMap<LogicalPath, Bytes>,
    ) -> Result<Self, ParserError> {
        if files.is_empty() {
            return Err(ParserError::InvalidProject {
                message: "project file map is empty".into(),
            });
        }
        if !files.contains_key(&main_file) {
            return Err(ParserError::InvalidProject {
                message: "main file is absent".into(),
            });
        }
        Ok(Self { main_file, files })
    }
    #[must_use]
    pub fn main_file(&self) -> &LogicalPath {
        &self.main_file
    }
    #[must_use]
    pub fn files(&self) -> &BTreeMap<LogicalPath, Bytes> {
        &self.files
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum DependencyResolution {
    ProjectFile(LogicalPath),
    ExternalTex,
    Missing(Vec<LogicalPath>),
    Dynamic,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectDependencyEdge {
    from: LogicalPath,
    kind: DependencyKind,
    raw: String,
    resolution: DependencyResolution,
    range: SourceRange,
}
impl ProjectDependencyEdge {
    #[must_use]
    pub fn from(&self) -> &LogicalPath {
        &self.from
    }
    #[must_use]
    pub const fn kind(&self) -> DependencyKind {
        self.kind
    }
    #[must_use]
    pub fn raw(&self) -> &str {
        &self.raw
    }
    #[must_use]
    pub fn resolution(&self) -> &DependencyResolution {
        &self.resolution
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum TexRequestKind {
    DocumentClass,
    Package,
}
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TexRequestResolution {
    from: LogicalPath,
    kind: TexRequestKind,
    name: String,
    resolution: DependencyResolution,
    range: SourceRange,
}
impl TexRequestResolution {
    #[must_use]
    pub fn from(&self) -> &LogicalPath {
        &self.from
    }
    #[must_use]
    pub const fn kind(&self) -> TexRequestKind {
        self.kind
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub fn resolution(&self) -> &DependencyResolution {
        &self.resolution
    }
    #[must_use]
    pub const fn range(&self) -> SourceRange {
        self.range
    }
}

/// Deterministically ordered project dependency edges.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DependencyGraph {
    edges: Vec<ProjectDependencyEdge>,
    tex_requests: Vec<TexRequestResolution>,
}
impl DependencyGraph {
    #[must_use]
    pub fn edges(&self) -> &[ProjectDependencyEdge] {
        &self.edges
    }
    #[must_use]
    pub fn tex_requests(&self) -> &[TexRequestResolution] {
        &self.tex_requests
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectDiagnostic {
    file: LogicalPath,
    diagnostic: ParserDiagnostic,
}
impl ProjectDiagnostic {
    #[must_use]
    pub fn file(&self) -> &LogicalPath {
        &self.file
    }
    #[must_use]
    pub fn diagnostic(&self) -> &ParserDiagnostic {
        &self.diagnostic
    }
}

/// Complete deterministic best-effort project analysis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProjectAnalysis {
    main_file: LogicalPath,
    files: BTreeMap<LogicalPath, FileAnalysis>,
    bibliographies: BTreeMap<LogicalPath, BibtexAnalysis>,
    dependency_graph: DependencyGraph,
    diagnostics: Vec<ProjectDiagnostic>,
}
impl ProjectAnalysis {
    #[must_use]
    pub fn main_file(&self) -> &LogicalPath {
        &self.main_file
    }
    #[must_use]
    pub fn files(&self) -> &BTreeMap<LogicalPath, FileAnalysis> {
        &self.files
    }
    #[must_use]
    pub fn bibliographies(&self) -> &BTreeMap<LogicalPath, BibtexAnalysis> {
        &self.bibliographies
    }
    #[must_use]
    pub const fn dependency_graph(&self) -> &DependencyGraph {
        &self.dependency_graph
    }
    #[must_use]
    pub fn diagnostics(&self) -> &[ProjectDiagnostic] {
        &self.diagnostics
    }
}

/// Synchronous deterministic multi-file analyzer.
///
/// Results are hints only; actual TeX execution remains the semantic authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProjectAnalyzer {
    limits: ParserLimits,
}
impl ProjectAnalyzer {
    #[must_use]
    pub const fn new(limits: ParserLimits) -> Self {
        Self { limits }
    }
    #[must_use]
    pub fn with_default_limits() -> Self {
        Self::new(ParserLimits::default())
    }
    /// Analyzes eligible files and resolves dependencies without filesystem access.
    ///
    /// # Errors
    /// Returns [`ParserError`] when parser infrastructure or checked offsets fail.
    #[allow(
        clippy::too_many_lines,
        reason = "project analysis preserves a linear deterministic flow"
    )]
    pub fn analyze(&self, source: &ProjectSource) -> Result<ProjectAnalysis, ParserError> {
        let mut files = BTreeMap::new();
        let mut bibliographies = BTreeMap::new();
        for (path, bytes) in &source.files {
            if path == &source.main_file || is_latex(path) {
                let session = ParserSession::new(path.clone(), bytes.clone(), self.limits)?;
                files.insert(path.clone(), session.analysis().clone());
            } else if is_bibtex(path) {
                let session = BibtexSession::new(path.clone(), bytes.clone())?;
                bibliographies.insert(path.clone(), session.analysis().clone());
            }
        }
        let mut graph = DependencyGraph::default();
        let mut diagnostics = Vec::new();
        for (path, analysis) in &files {
            for diagnostic in analysis.diagnostics() {
                diagnostics.push(ProjectDiagnostic {
                    file: path.clone(),
                    diagnostic: diagnostic.clone(),
                });
            }
            for request in analysis.dependencies() {
                let raw = request.target().raw().to_owned();
                let resolution = match request.target() {
                    DependencyTarget::Dynamic(_) => DependencyResolution::Dynamic,
                    DependencyTarget::Static(raw) => {
                        resolve_dependency(path, request.kind(), raw, &source.files)
                    }
                };
                if let DependencyResolution::Missing(_) = &resolution {
                    diagnostics.push(ProjectDiagnostic {
                        file: path.clone(),
                        diagnostic: ParserDiagnostic::new(
                            DiagnosticSeverity::Warning,
                            DiagnosticCode::MissingProjectDependency,
                            format!("project dependency not found: {raw}"),
                            Some(request.range()),
                        ),
                    });
                }
                graph.edges.push(ProjectDependencyEdge {
                    from: path.clone(),
                    kind: request.kind(),
                    raw,
                    resolution,
                    range: request.range(),
                });
            }
            for class in analysis.document_classes() {
                graph.tex_requests.push(resolve_tex_request(
                    path,
                    TexRequestKind::DocumentClass,
                    class.name(),
                    "cls",
                    class.range(),
                    &source.files,
                ));
            }
            for package in analysis.packages() {
                graph.tex_requests.push(resolve_tex_request(
                    path,
                    TexRequestKind::Package,
                    package.name(),
                    "sty",
                    package.range(),
                    &source.files,
                ));
            }
        }
        graph.edges.sort_by(|a, b| {
            (&a.from, a.range.start_byte(), a.kind, &a.raw).cmp(&(
                &b.from,
                b.range.start_byte(),
                b.kind,
                &b.raw,
            ))
        });
        graph.tex_requests.sort_by(|a, b| {
            (&a.from, a.range.start_byte(), a.kind, &a.name).cmp(&(
                &b.from,
                b.range.start_byte(),
                b.kind,
                &b.name,
            ))
        });
        add_cycle_diagnostics(&graph, &mut diagnostics);
        add_label_diagnostics(&files, &mut diagnostics);
        add_bibliography_diagnostics(
            &source.main_file,
            &files,
            &bibliographies,
            &graph,
            &mut diagnostics,
        );
        diagnostics.sort_by(|a, b| {
            (&a.file, diagnostic_key(&a.diagnostic)).cmp(&(&b.file, diagnostic_key(&b.diagnostic)))
        });
        diagnostics.dedup();
        Ok(ProjectAnalysis {
            main_file: source.main_file.clone(),
            files,
            bibliographies,
            dependency_graph: graph,
            diagnostics,
        })
    }
}
fn diagnostic_key(diagnostic: &ParserDiagnostic) -> (u64, u64, DiagnosticCode) {
    (
        diagnostic.range().map_or(u64::MAX, SourceRange::start_byte),
        diagnostic.range().map_or(u64::MAX, SourceRange::end_byte),
        diagnostic.code(),
    )
}
fn is_latex(path: &LogicalPath) -> bool {
    path.extension().is_some_and(|extension| {
        matches!(
            extension.to_ascii_lowercase().as_str(),
            "tex" | "ltx" | "sty" | "cls"
        )
    })
}
fn is_bibtex(path: &LogicalPath) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("bib"))
}
fn parent(path: &LogicalPath) -> &str {
    path.as_str()
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent)
}
fn normalized(base: &str, raw: &str) -> Option<LogicalPath> {
    if raw.starts_with('/') || raw.contains('\\') {
        return None;
    }
    let mut parts: Vec<&str> = base.split('/').filter(|part| !part.is_empty()).collect();
    for part in raw.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            value => parts.push(value),
        }
    }
    LogicalPath::parse(&parts.join("/")).ok()
}
fn has_extension(raw: &str) -> bool {
    raw.rsplit('/').next().is_some_and(|name| {
        name.rsplit_once('.')
            .is_some_and(|(_, extension)| !extension.is_empty())
    })
}
fn candidates(from: &LogicalPath, kind: DependencyKind, raw: &str) -> Vec<LogicalPath> {
    let suffixes: &[&str] = match (kind, has_extension(raw)) {
        (_, true) => &[""],
        (DependencyKind::Input | DependencyKind::Include | DependencyKind::Subfile, false) => {
            &["", ".tex"]
        }
        (DependencyKind::Graphics, false) => &["", ".pdf", ".png", ".jpg", ".jpeg", ".eps", ".svg"],
        (DependencyKind::Bibliography, false) => &["", ".bib"],
    };
    suffixes
        .iter()
        .filter_map(|suffix| normalized(parent(from), &format!("{raw}{suffix}")))
        .collect()
}
fn resolve_dependency(
    from: &LogicalPath,
    kind: DependencyKind,
    raw: &str,
    files: &BTreeMap<LogicalPath, Bytes>,
) -> DependencyResolution {
    let candidates = candidates(from, kind, raw);
    candidates
        .iter()
        .find(|candidate| files.contains_key(*candidate))
        .cloned()
        .map_or_else(
            || DependencyResolution::Missing(candidates),
            DependencyResolution::ProjectFile,
        )
}
fn resolve_tex_request(
    from: &LogicalPath,
    kind: TexRequestKind,
    name: &str,
    extension: &str,
    range: SourceRange,
    files: &BTreeMap<LogicalPath, Bytes>,
) -> TexRequestResolution {
    let candidate = normalized(parent(from), &format!("{name}.{extension}"));
    let resolution = candidate.filter(|path| files.contains_key(path)).map_or(
        DependencyResolution::ExternalTex,
        DependencyResolution::ProjectFile,
    );
    TexRequestResolution {
        from: from.clone(),
        kind,
        name: name.to_owned(),
        resolution,
        range,
    }
}
fn add_cycle_diagnostics(graph: &DependencyGraph, diagnostics: &mut Vec<ProjectDiagnostic>) {
    let mut adjacency: BTreeMap<LogicalPath, BTreeSet<LogicalPath>> = BTreeMap::new();
    for edge in &graph.edges {
        if matches!(
            edge.kind,
            DependencyKind::Input | DependencyKind::Include | DependencyKind::Subfile
        ) {
            if let DependencyResolution::ProjectFile(target) = &edge.resolution {
                adjacency
                    .entry(edge.from.clone())
                    .or_default()
                    .insert(target.clone());
            }
        }
    }
    let mut visited = BTreeSet::new();
    let mut active = BTreeSet::new();
    let mut reported = BTreeSet::new();
    for node in adjacency.keys() {
        detect_cycle(
            node,
            &adjacency,
            &mut visited,
            &mut active,
            &mut reported,
            diagnostics,
        );
    }
}
fn detect_cycle(
    node: &LogicalPath,
    adjacency: &BTreeMap<LogicalPath, BTreeSet<LogicalPath>>,
    visited: &mut BTreeSet<LogicalPath>,
    active: &mut BTreeSet<LogicalPath>,
    reported: &mut BTreeSet<LogicalPath>,
    diagnostics: &mut Vec<ProjectDiagnostic>,
) {
    if active.contains(node) {
        if reported.insert(node.clone()) {
            diagnostics.push(ProjectDiagnostic {
                file: node.clone(),
                diagnostic: ParserDiagnostic::new(
                    DiagnosticSeverity::Warning,
                    DiagnosticCode::DependencyCycle,
                    "source dependency cycle detected",
                    None,
                ),
            });
        }
        return;
    }
    if !visited.insert(node.clone()) {
        return;
    }
    active.insert(node.clone());
    if let Some(targets) = adjacency.get(node) {
        for target in targets {
            detect_cycle(target, adjacency, visited, active, reported, diagnostics);
        }
    }
    active.remove(node);
}
fn add_label_diagnostics(
    files: &BTreeMap<LogicalPath, FileAnalysis>,
    diagnostics: &mut Vec<ProjectDiagnostic>,
) {
    let mut labels: BTreeMap<&str, Vec<(&LogicalPath, SourceRange)>> = BTreeMap::new();
    for (path, analysis) in files {
        for label in analysis.labels() {
            labels
                .entry(label.key())
                .or_default()
                .push((path, label.range()));
        }
    }
    for (key, locations) in &labels {
        if locations.len() > 1 {
            for (path, range) in locations {
                diagnostics.push(ProjectDiagnostic {
                    file: (*path).clone(),
                    diagnostic: ParserDiagnostic::new(
                        DiagnosticSeverity::Warning,
                        DiagnosticCode::DuplicateLabel,
                        format!("duplicate label: {key}"),
                        Some(*range),
                    ),
                });
            }
        }
    }
    for (path, analysis) in files {
        for reference in analysis.references() {
            if !labels.contains_key(reference.key()) {
                diagnostics.push(ProjectDiagnostic {
                    file: path.clone(),
                    diagnostic: ParserDiagnostic::new(
                        DiagnosticSeverity::Warning,
                        DiagnosticCode::UnresolvedReference,
                        format!("unresolved reference: {}", reference.key()),
                        Some(reference.range()),
                    ),
                });
            }
        }
    }
}

fn add_bibliography_diagnostics(
    main_file: &LogicalPath,
    files: &BTreeMap<LogicalPath, FileAnalysis>,
    bibliographies: &BTreeMap<LogicalPath, BibtexAnalysis>,
    graph: &DependencyGraph,
    diagnostics: &mut Vec<ProjectDiagnostic>,
) {
    let mut reachable_sources = BTreeSet::from([main_file.clone()]);
    loop {
        let before = reachable_sources.len();
        for edge in &graph.edges {
            if reachable_sources.contains(&edge.from)
                && matches!(
                    edge.kind,
                    DependencyKind::Input | DependencyKind::Include | DependencyKind::Subfile
                )
            {
                if let DependencyResolution::ProjectFile(target) = &edge.resolution {
                    reachable_sources.insert(target.clone());
                }
            }
        }
        if reachable_sources.len() == before {
            break;
        }
    }
    let mut reachable_bibs = BTreeSet::new();
    let mut has_dynamic_bibliography = false;
    for edge in &graph.edges {
        if edge.kind == DependencyKind::Bibliography && reachable_sources.contains(&edge.from) {
            match &edge.resolution {
                DependencyResolution::ProjectFile(path) => {
                    reachable_bibs.insert(path.clone());
                }
                DependencyResolution::Dynamic => has_dynamic_bibliography = true,
                _ => {}
            }
        }
    }
    let mut definitions: BTreeMap<&str, Vec<(&LogicalPath, SourceRange)>> = BTreeMap::new();
    for path in &reachable_bibs {
        if let Some(analysis) = bibliographies.get(path) {
            for diagnostic in analysis.diagnostics() {
                diagnostics.push(ProjectDiagnostic {
                    file: path.clone(),
                    diagnostic: diagnostic.clone(),
                });
            }
            for entry in analysis.entries() {
                definitions
                    .entry(entry.key())
                    .or_default()
                    .push((path, entry.range()));
            }
        }
    }
    for (key, locations) in &definitions {
        if locations.len() > 1 {
            for (path, range) in locations {
                diagnostics.push(ProjectDiagnostic {
                    file: (*path).clone(),
                    diagnostic: ParserDiagnostic::new(
                        DiagnosticSeverity::Warning,
                        DiagnosticCode::DuplicateBibtexKey,
                        format!("duplicate BibTeX key: {key}"),
                        Some(*range),
                    ),
                });
            }
        }
    }
    if !has_dynamic_bibliography {
        for path in &reachable_sources {
            if let Some(analysis) = files.get(path) {
                for citation in analysis.citations() {
                    for key in citation.keys() {
                        if !key.is_empty() && key != "*" && !definitions.contains_key(key.as_str())
                        {
                            diagnostics.push(ProjectDiagnostic {
                                file: path.clone(),
                                diagnostic: ParserDiagnostic::new(
                                    DiagnosticSeverity::Warning,
                                    DiagnosticCode::UnresolvedCitation,
                                    format!("unresolved citation: {key}"),
                                    Some(citation.range()),
                                ),
                            });
                        }
                    }
                }
            }
        }
    }
}
