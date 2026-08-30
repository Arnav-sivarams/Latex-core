#![cfg(feature = "docker-tests")]

use blob_store::{BlobStore, FsBlobStore, FsBlobStoreConfig};
use bytes::Bytes;
use compiler::{
    CompileLimits, CompileStatus, CompilerConfig, CompilerService, DockerCliRuntime, SyncTexIndex,
};
use core_types::{
    ArtifactKind, FileEntryV1, LogicalPath, ShellPolicy, TexEngine, WorkspaceManifestV1,
};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

async fn compile_with_policy(
    files: &[(&str, &[u8])],
    main: &str,
    engine: TexEngine,
    timeout: Duration,
    shell_policy: ShellPolicy,
) -> (compiler::CompileExecution, tempfile::TempDir) {
    let image = std::env::var("LATEX_CORE_TEXLIVE_IMAGE").expect("LATEX_CORE_TEXLIVE_IMAGE must contain an immutable image reference when docker-tests is enabled");
    let directory = tempfile::tempdir().expect("tempdir");
    let store = FsBlobStore::open(
        directory.path().join("blobs"),
        FsBlobStoreConfig::development_default(),
    )
    .await
    .expect("store");
    let mut entries = BTreeMap::new();
    for (path, bytes) in files {
        let put = store.put(Bytes::copy_from_slice(bytes)).await.expect("put");
        entries.insert(
            LogicalPath::parse(path).expect("path"),
            FileEntryV1 {
                blob_hash: put.hash(),
                size_bytes: put.size_bytes(),
            },
        );
    }
    let manifest = WorkspaceManifestV1::new(LogicalPath::parse(main).expect("main"), entries)
        .expect("manifest");
    let snapshot = manifest.snapshot_id().expect("snapshot");
    let limits = CompileLimits::new(
        timeout,
        4 * 1024 * 1024,
        4 * 1024 * 1024,
        128 * 1024 * 1024,
        256 * 1024 * 1024,
        1024 * 1024 * 1024,
        128,
        1.0,
    )
    .expect("limits");
    let service = CompilerService::new(
        Arc::new(store),
        DockerCliRuntime::new(image).expect("runtime"),
        CompilerConfig::new(limits),
    )
    .expect("probe image");
    let execution = service
        .compile(snapshot, &manifest, engine, shell_policy, true)
        .await
        .expect("compiler infrastructure");
    (execution, directory)
}

async fn compile(
    files: &[(&str, &[u8])],
    main: &str,
    engine: TexEngine,
    timeout: Duration,
) -> (compiler::CompileExecution, tempfile::TempDir) {
    compile_with_policy(files, main, engine, timeout, ShellPolicy::Safe).await
}
fn pdf(execution: &compiler::CompileExecution) {
    let artifact = execution
        .artifacts()
        .iter()
        .find(|a| a.kind() == ArtifactKind::Pdf)
        .expect("PDF artifact");
    assert!(artifact.bytes().starts_with(b"%PDF-"));
    assert!(
        execution
            .artifacts()
            .iter()
            .any(|a| a.kind() == ArtifactKind::Log)
    );
    assert!(
        execution
            .artifacts()
            .iter()
            .any(|a| a.kind() == ArtifactKind::Fls)
    );
}

#[tokio::test]
async fn representative_v2_pdf_log_and_nonempty_synctex() {
    let source = b"\\documentclass{article}\n\\begin{document}\n\nS5 representative paragraph for linked review.\n\\end{document}\n";
    let (execution, _) = compile(
        &[("paper.tex", source)],
        "paper.tex",
        TexEngine::PdfLatex,
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(execution.status(), CompileStatus::Succeeded);
    pdf(&execution);
    let synctex = execution
        .artifacts()
        .iter()
        .find(|artifact| artifact.kind() == ArtifactKind::Synctex)
        .expect("SyncTeX artifact");
    assert!(!synctex.bytes().is_empty());
    assert!(synctex.logical_name().as_str().ends_with(".synctex.gz"));
    let index = SyncTexIndex::from_gzip(synctex.bytes()).expect("read representative SyncTeX");
    let forward = index
        .forward("paper.tex", 4, 0)
        .expect("forward mapping for normal paragraph");
    assert_eq!(forward.page, 1);
    assert!(forward.x.is_finite() && forward.y.is_finite());
    let inverse = index
        .inverse(forward.page, forward.x, forward.y)
        .expect("inverse mapping from projected position");
    assert!(inverse.source_path.ends_with("paper.tex"));
    assert!(inverse.line > 0);
    assert!(index.forward("intentionally-missing.tex", 999, 0).is_none());
    assert!(index.inverse(999, -1.0, -1.0).is_none());
}

#[tokio::test]
async fn basic_pdf_and_all_four_engines() {
    let source = br"\documentclass{article}\begin{document}Hello\end{document}";
    for engine in [
        TexEngine::Latex,
        TexEngine::PdfLatex,
        TexEngine::LuaLatex,
        TexEngine::XeLatex,
    ] {
        let (execution, _) = compile(
            &[("main.tex", source)],
            "main.tex",
            engine,
            Duration::from_secs(60),
        )
        .await;
        assert_eq!(
            execution.status(),
            CompileStatus::Succeeded,
            "{}",
            String::from_utf8_lossy(execution.stderr())
        );
        pdf(&execution);
        assert!(
            execution
                .artifacts()
                .iter()
                .any(|a| a.kind() == ArtifactKind::Synctex)
        );
        assert!(
            execution
                .environment_id()
                .as_str()
                .starts_with("texlive-2026-sha256-")
        );
    }
}

#[tokio::test]
async fn orchestration_packages_local_files_and_paths() {
    let fixtures: Vec<(&str, Vec<(&str, &[u8])>, &str, TexEngine)> = vec![
        ("multi", vec![("main.tex", br"\documentclass{article}\begin{document}\input{chapters/one}\end{document}"), ("chapters/one.tex", b"Included")], "main.tex", TexEngine::PdfLatex),
        ("tikz", vec![("main.tex", br"\documentclass{article}\usepackage{tikz}\begin{document}\begin{tikzpicture}\draw (0,0)--(1,1);\end{tikzpicture}\end{document}")], "main.tex", TexEngine::PdfLatex),
        ("beamer", vec![("main.tex", br"\documentclass{beamer}\begin{document}\begin{frame}Hi\end{frame}\end{document}")], "main.tex", TexEngine::PdfLatex),
        ("book", vec![("main.tex", br"\documentclass{book}\begin{document}\include{chapters/one}\end{document}"), ("chapters/one.tex", br"\chapter{One}Included")], "main.tex", TexEngine::PdfLatex),
        ("ieee", vec![("main.tex", br"\documentclass{IEEEtran}\begin{document}Hi\end{document}")], "main.tex", TexEngine::PdfLatex),
        ("acmart", vec![("main.tex", br"\documentclass{acmart}\begin{document}Hi\end{document}")], "main.tex", TexEngine::PdfLatex),
        ("local", vec![("space dir/main file.tex", br"\documentclass{myclass}\usepackage{mystyle}\begin{document}\hello\end{document}"), ("space dir/myclass.cls", br"\LoadClass{article}"), ("space dir/mystyle.sty", br"\newcommand\hello{Local}")], "space dir/main file.tex", TexEngine::PdfLatex),
        ("unicode", vec![("main.tex", "\\documentclass{article}\\begin{document}Unicode 文書\\end{document}".as_bytes())], "main.tex", TexEngine::LuaLatex),
    ];
    for (name, files, main, engine) in fixtures {
        let (execution, _) = compile(&files, main, engine, Duration::from_secs(60)).await;
        assert_eq!(
            execution.status(),
            CompileStatus::Succeeded,
            "{name}: {}",
            String::from_utf8_lossy(execution.stderr())
        );
        pdf(&execution);
    }
}

#[tokio::test]
async fn bibtex_biber_and_makeindex_complete() {
    let cases: Vec<Vec<(&str, &[u8])>> = vec![
        vec![("main.tex", br"\documentclass{article}\begin{document}\cite{x}\bibliographystyle{plain}\bibliography{refs}\end{document}"), ("refs.bib", br"@book{x,title={X},author={A},year={2026}}")],
        vec![("main.tex", br"\documentclass{article}\usepackage[backend=biber]{biblatex}\addbibresource{refs.bib}\begin{document}\cite{x}\printbibliography\end{document}"), ("refs.bib", br"@book{x,title={X},author={A},date={2026}}")],
        vec![("main.tex", br"\documentclass{article}\usepackage{makeidx}\makeindex\begin{document}Word\index{word}\printindex\end{document}")],
        vec![("main.tex", br"\documentclass{article}\begin{document}\cite{x}\bibliographystyle{local}\bibliography{refs}\end{document}"), ("refs.bib", br"@book{x,title={X},author={A},year={2026}}"), ("local.bst", br"ENTRY{ }{}{}FUNCTION {begin.bib}{ }READ EXECUTE {begin.bib}" )],
        vec![("main.tex", br"\documentclass{article}\usepackage{glossaries}\makeglossaries\newglossaryentry{term}{name={term},description={A term}}\begin{document}\gls{term}\printglossaries\end{document}")],
    ];
    for files in cases {
        let (execution, _) = compile(
            &files,
            "main.tex",
            TexEngine::PdfLatex,
            Duration::from_secs(60),
        )
        .await;
        assert_eq!(
            execution.status(),
            CompileStatus::Succeeded,
            "{}",
            String::from_utf8_lossy(execution.stderr())
        );
        pdf(&execution);
    }
}

#[tokio::test]
async fn failures_shell_and_latexmkrc_are_controlled() {
    let files: &[(&str, &[u8])] = &[("main.tex", br"\documentclass{article}\begin{document}\immediate\write18{touch /work/.latex-core-out/PWNED_WRITE18}ok\end{document}"), (".latexmkrc", br"die 'hostile project latexmkrc executed'; system('touch /work/.latex-core-out/PWNED_LATEXMKRC');")];
    let (execution, _) = compile(
        files,
        "main.tex",
        TexEngine::PdfLatex,
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(
        execution.status(),
        CompileStatus::Succeeded,
        "classical EPS: {}",
        String::from_utf8_lossy(execution.stderr())
    );
    pdf(&execution);
    assert!(!execution.artifacts().iter().any(|a| matches!(
        a.logical_name().as_str(),
        "PWNED_WRITE18" | "PWNED_LATEXMKRC"
    )));
    let (malformed, _) = compile(
        &[(
            "main.tex",
            br"\documentclass{article}\begin{document}\undefinedcommand",
        )],
        "main.tex",
        TexEngine::PdfLatex,
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(malformed.status(), CompileStatus::Failed);
    assert!(
        malformed
            .artifacts()
            .iter()
            .any(|a| a.kind() == ArtifactKind::Log)
    );
    let (missing, _) = compile(
        &[(
            "main.tex",
            br"\documentclass{article}\begin{document}\input{absent}\end{document}",
        )],
        "main.tex",
        TexEngine::PdfLatex,
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(missing.status(), CompileStatus::Failed);
}

#[tokio::test]
async fn classical_latex_eps_and_nested_local_dependencies_succeed() {
    let eps = br"%!PS-Adobe-3.0 EPSF-3.0
%%BoundingBox: 0 0 10 10
newpath 0 0 moveto 10 10 lineto stroke
showpage
";
    let (execution, _) = compile(
        &[
            ("main.tex", br"\documentclass{article}\usepackage{graphicx}\begin{document}\includegraphics{assets/line}\end{document}"),
            ("assets/line.eps", eps),
        ],
        "main.tex",
        TexEngine::Latex,
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(
        execution.status(),
        CompileStatus::Succeeded,
        "classical EPS: {}",
        String::from_utf8_lossy(execution.stderr())
    );
    pdf(&execution);
    let (execution, _) = compile(
        &[
            ("nested/main.tex", br"\documentclass{local/class}\usepackage{local/style}\begin{document}\localhello\end{document}"),
            ("nested/local/class.cls", br"\LoadClass{article}"),
            ("nested/local/style.sty", br"\newcommand\localhello{Nested}"),
        ],
        "nested/main.tex",
        TexEngine::PdfLatex,
        Duration::from_secs(60),
    )
    .await;
    assert_eq!(
        execution.status(),
        CompileStatus::Succeeded,
        "nested local dependencies: {}",
        String::from_utf8_lossy(execution.stderr())
    );
    pdf(&execution);
}

#[tokio::test]
async fn png_and_unicode_xelatex_projects_succeed() {
    let png: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 8, 215, 99, 248, 207, 192, 240, 31,
        0, 5, 0, 1, 255, 114, 156, 82, 103, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    let (execution, _) = compile(
        &[("main.tex", br"\documentclass{article}\usepackage{graphicx}\begin{document}\includegraphics{pixel.png}\end{document}"), ("pixel.png", png)],
        "main.tex",
        TexEngine::PdfLatex,
        Duration::from_secs(60),
    ).await;
    assert_eq!(
        execution.status(),
        CompileStatus::Succeeded,
        "PNG pdfLaTeX: {}",
        String::from_utf8_lossy(execution.stderr())
    );
    pdf(&execution);
    let (execution, _) = compile(
        &[("main.tex", "\\documentclass{article}\\usepackage{fontspec}\\begin{document}Unicode 文書\\end{document}".as_bytes())],
        "main.tex",
        TexEngine::XeLatex,
        Duration::from_secs(60),
    ).await;
    assert_eq!(
        execution.status(),
        CompileStatus::Succeeded,
        "Unicode XeLaTeX: {}",
        String::from_utf8_lossy(execution.stderr())
    );
    pdf(&execution);
}

#[tokio::test]
async fn infinite_tex_times_out() {
    let (execution, _) = compile(
        &[(
            "main.tex",
            br"\documentclass{article}\begin{document}\loop\iftrue\repeat",
        )],
        "main.tex",
        TexEngine::PdfLatex,
        Duration::from_secs(3),
    )
    .await;
    assert_eq!(execution.status(), CompileStatus::TimedOut);
}
