#![allow(clippy::unwrap_used, reason = "tests use known-valid fixtures")]
use bytes::Bytes;
use core_types::LogicalPath;
use latex_parser::*;

fn parse(source: &'static [u8]) -> ParserSession {
    ParserSession::with_default_limits(
        LogicalPath::parse("main.tex").unwrap(),
        Bytes::from_static(source),
    )
    .unwrap()
}

#[test]
fn vendored_grammar_bootstrap_regression() {
    let session = parse(b"\\documentclass{article}\n\\begin{document}\nHello\\label{sec_first_test}\n\\MyUniversityCommand{hello}\n\\end{document}");
    assert_eq!(session.analysis().document_classes()[0].name(), "article");
    assert!(
        session
            .analysis()
            .environments()
            .iter()
            .any(|item| item.name() == "document")
    );
    assert!(
        session
            .analysis()
            .labels()
            .iter()
            .any(|item| item.key() == "sec_first_test")
    );
    assert!(
        session
            .analysis()
            .commands()
            .iter()
            .any(|item| item.name() == "MyUniversityCommand")
    );
    assert!(
        ParserSession::with_default_limits(
            LogicalPath::parse("main.tex").unwrap(),
            Bytes::from_static(b"\\section{unfinished")
        )
        .is_ok()
    );
}

#[test]
fn complete_semantics_are_structural_and_deterministic() {
    let source = br"% ordinary comment
% !TeX root = main.tex
\documentclass[11pt]{article}
\usepackage{amsmath, graphicx}
\newcommand{\hello}[1]{Hello #1}
\begin{document}
\section{Introduction}\label{sec:intro}
\subsection{Prior Work}
See \ref{sec:intro}, \eqref{eq:x}, and \autoref{sec:intro}.
\citep[see][p. 2]{alpha,beta}
\input{chapters/one}\include{appendix}\includegraphics[width=2cm]{figures/chart}
\bibliography{refs,extra}\addbibresource{more.bib}
\begin{equation}\label{eq:x}x=1\end{equation}
$y=2$ and \[z=3\]
\MyUniversityThesisHeading{AI Systems}
\end{document}
";
    let session = parse(source);
    let a = session.analysis();
    assert_eq!(a.document_classes()[0].name(), "article");
    assert_eq!(
        a.packages()
            .iter()
            .map(PackageRequest::name)
            .collect::<Vec<_>>(),
        ["amsmath", "graphicx"]
    );
    assert_eq!(
        a.sections().iter().map(Section::title).collect::<Vec<_>>(),
        ["Introduction", "Prior Work"]
    );
    assert!(
        a.environments()
            .iter()
            .any(|item| item.name() == "document")
    );
    assert!(a.labels().iter().any(|item| item.key() == "sec:intro"));
    assert!(
        a.references()
            .iter()
            .any(|item| item.kind() == ReferenceKind::EqRef && item.key() == "eq:x")
    );
    assert!(
        a.citations()
            .iter()
            .any(|item| item.command() == "citep" && item.keys() == ["alpha", "beta"])
    );
    assert!(
        a.dependencies()
            .iter()
            .any(|item| item.kind() == DependencyKind::Graphics
                && item.target().raw() == "figures/chart")
    );
    assert!(
        a.dependencies().iter().any(
            |item| item.kind() == DependencyKind::Bibliography && item.target().raw() == "refs"
        )
    );
    assert!(a.macros().iter().any(|item| item.name() == "hello"));
    assert!(
        a.commands()
            .iter()
            .any(|item| item.name() == "MyUniversityThesisHeading")
    );
    assert!(
        a.math_regions()
            .iter()
            .any(|item| item.kind() == MathKind::Inline)
    );
    assert!(
        a.comments()
            .iter()
            .any(|item| item.text().contains("ordinary comment"))
    );
    assert!(
        a.directives()
            .iter()
            .any(|item| item.key() == "root" && item.value() == "main.tex")
    );
    assert!(
        a.sections()
            .windows(2)
            .all(|window| window[0].range().start_byte() <= window[1].range().start_byte())
    );
}

#[test]
fn templates_comments_and_verbatim_preserve_expected_structure() {
    for (class, extra) in [
        ("article", ""),
        ("IEEEtran", ""),
        ("beamer", "\\begin{frame}x\\end{frame}"),
        (
            "article",
            "\\usepackage{tikz}\\begin{tikzpicture}x\\end{tikzpicture}",
        ),
        ("vitthesis", ""),
    ] {
        let text = format!("\\documentclass{{{class}}}\\begin{{document}}{extra}\\end{{document}}");
        let session = ParserSession::with_default_limits(
            LogicalPath::parse("main.tex").unwrap(),
            Bytes::from(text),
        )
        .unwrap();
        assert_eq!(session.analysis().document_classes()[0].name(), class);
    }
    let commented = parse(b"% \\input{evil.tex}\n% \\label{fake}\n% \\section{Fake}\n\\begin{verbatim}\n\\input{not-real.tex}\n\\label{not-real}\n\\end{verbatim}");
    assert!(commented.analysis().dependencies().is_empty());
    assert!(commented.analysis().labels().is_empty());
    assert!(commented.analysis().sections().is_empty());
    assert!(commented.analysis().comments().len() >= 3);
}
