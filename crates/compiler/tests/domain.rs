use compiler::{
    CompileContainerRequest, CompileLimits, CompilerError, DockerCliRuntime, profile_for,
};
use core_types::{LogicalPath, ShellPolicy, TexEngine};
use std::{ffi::OsString, path::PathBuf, time::Duration};

#[test]
fn limits_profiles_and_image_references_are_validated() {
    assert!(CompileLimits::new(Duration::ZERO, 1, 1, 1, 1, 1, 1, 1.0).is_err());
    assert_eq!(
        profile_for(ShellPolicy::Safe).expect("safe").as_str(),
        "safe-v1"
    );
    assert_eq!(
        profile_for(ShellPolicy::Restricted)
            .expect("restricted")
            .as_str(),
        "restricted-v1"
    );
    assert!(matches!(
        profile_for(ShellPolicy::Compatibility),
        Err(CompilerError::UnsupportedShellPolicy { .. })
    ));
    let digest = "a".repeat(64);
    for image in [
        format!("sha256:{digest}"),
        format!("repository@sha256:{digest}"),
    ] {
        assert!(DockerCliRuntime::new(image).is_ok());
    }
    for image in [
        "latest".to_owned(),
        "repository:latest".to_owned(),
        "repository:tag".to_owned(),
        format!("sha256:{}", "a".repeat(63)),
        format!("sha256:{}", "a".repeat(65)),
        format!("sha256:{}", "A".repeat(64)),
        format!("sha256:{}", "g".repeat(64)),
        format!("repository@sha256:{}", "a".repeat(63)),
        format!("repository@sha256:{}", "a".repeat(65)),
        format!("repository@sha256:{}", "A".repeat(64)),
        format!("repository@sha256:{}", "g".repeat(64)),
        format!("repo@@sha256:{}", "a".repeat(64)),
        format!("repo name@sha256:{}", "a".repeat(64)),
        format!("repo\nname@sha256:{}", "a".repeat(64)),
        String::new(),
        "sha256:".to_owned(),
        "repository@sha256:".to_owned(),
    ] {
        assert!(
            matches!(
                DockerCliRuntime::new(image),
                Err(CompilerError::InvalidImageReference { .. })
            ),
            "unexpectedly accepted an invalid immutable image reference"
        );
    }
}

#[test]
fn docker_arguments_match_the_frozen_runtime_contract_exactly() {
    let image = format!("repository@sha256:{}", "b".repeat(64));
    let runtime = DockerCliRuntime::new(&image).expect("immutable image");
    let limits = CompileLimits::development_default();
    let request = CompileContainerRequest::new(
        PathBuf::from("/tmp/one-project"),
        LogicalPath::parse("path with spaces/main.tex").expect("logical path"),
        TexEngine::PdfLatex,
        ShellPolicy::Safe,
        true,
        limits,
    )
    .with_execution_id("job-42");
    let args = runtime.compile_args(&request, "fixed-name");
    let expected = [
        "run",
        "--rm",
        "--name=fixed-name",
        "--label=latex-core.application=latex-core",
        "--label=latex-core.job-id=job-42",
        "--pull=never",
        "--network=none",
        "--read-only",
        "--cap-drop=ALL",
        "--security-opt=no-new-privileges",
        "--pids-limit=128",
        "--memory=1073741824",
        "--memory-swap=1073741824",
        "--cpus=1",
        "--user=10001:10001",
        "--tmpfs=/tmp:rw,noexec,nosuid,nodev,size=256m,mode=1777",
        "--env=HOME=/tmp/home",
        "--env=TEXMFHOME=/tmp/texmf-home",
        "--env=TEXMFVAR=/tmp/texmf-var",
        "--env=TEXMFCONFIG=/tmp/texmf-config",
        "--mount=type=bind,source=/tmp/one-project,target=/work",
        &image,
        "--engine",
        "pdflatex",
        "--shell-policy",
        "safe",
        "--synctex",
        "1",
        "--main",
        "path with spaces/main.tex",
    ]
    .into_iter()
    .map(OsString::from)
    .collect::<Vec<_>>();
    assert_eq!(args, expected);
    assert_eq!(
        args.iter()
            .filter(|arg| arg.to_string_lossy().starts_with("--mount="))
            .count(),
        1
    );
}
