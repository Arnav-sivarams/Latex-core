//! Diagnostics exposed by Writer's Problems panel, independently of parser analysis.

use core_types::LogicalPath;
use latex_parser::ProjectDiagnostic;
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

pub fn problems_response(
    diagnostics: &[ProjectDiagnostic],
    file_ids: &BTreeMap<LogicalPath, Uuid>,
) -> Vec<Value> {
    diagnostics
        .iter()
        .map(|item| {
            serde_json::json!({
                "severity":super::diagnostic_severity_name(item.diagnostic().severity()),
                "code":format!("{:?}", item.diagnostic().code()),
                "message":item.diagnostic().message(),
                "file_id":file_ids.get(item.file()),"path":item.file(),
                "range":item.diagnostic().range().map(super::source_range_json),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use latex_parser::DiagnosticCode;

    #[test]
    fn writer_problems_preserve_real_errors_and_missing_dependencies() {
        let main = LogicalPath::parse("Full_Report_template_v1.0/Full_Report_v1.0.tex")
            .expect("valid template path");
        let source = latex_parser::ProjectSource::new(
            main.clone(),
            BTreeMap::from([(
                main.clone(),
                bytes::Bytes::from_static(
                    br"\documentclass{article}
\begin{document}
\includegraphics{images/sample-graph.pdf}
\ref{missing-label}
\section{broken
",
                ),
            )]),
        )
        .expect("valid project source");
        let analysis = latex_parser::ProjectAnalyzer::with_default_limits()
            .analyze(&source)
            .expect("best-effort parser analysis");
        for code in [
            DiagnosticCode::SyntaxError,
            DiagnosticCode::MissingProjectDependency,
            DiagnosticCode::UnresolvedReference,
        ] {
            assert!(
                analysis
                    .diagnostics()
                    .iter()
                    .any(|item| item.diagnostic().code() == code),
                "fixture must produce {code:?}"
            );
        }
        let file_id = Uuid::from_u128(1);
        let result = problems_response(analysis.diagnostics(), &BTreeMap::from([(main, file_id)]));
        assert!(result.iter().any(|item| item["code"] == "SyntaxError"
            && item["severity"] == "error"
            && item["file_id"] == file_id.to_string()));
        assert!(
            result
                .iter()
                .any(|item| item["code"] == "UnresolvedReference")
        );
        assert!(
            result
                .iter()
                .any(|item| item["code"] == "MissingProjectDependency")
        );
        assert!(
            serde_json::to_string(&result)
                .expect("serializable response")
                .contains("project dependency not found")
        );
    }
}
