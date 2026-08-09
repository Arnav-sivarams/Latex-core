use crate::ParserError;
use tree_sitter::{Language, Query};

pub(crate) const SEMANTIC_QUERY: &str = r"
[
 (class_include) (package_include)
 (part) (chapter) (section) (subsection) (subsubsection) (paragraph) (subparagraph)
 (generic_environment) (label_definition) (label_reference) (citation)
 (latex_include) (graphics_include) (bibtex_include) (biblatex_include)
 (new_command_definition) (old_command_definition) (generic_command)
 (inline_formula) (displayed_equation) (comment) (line_comment)
] @semantic
";

pub(crate) fn compile(language: &Language) -> Result<Query, ParserError> {
    Query::new(language, SEMANTIC_QUERY).map_err(|error| ParserError::QueryInitialization {
        message: error.to_string(),
    })
}
