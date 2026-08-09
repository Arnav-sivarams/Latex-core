#![allow(clippy::unwrap_used, reason = "controlled fixtures")]
use std::{ffi::OsStr, path::Path, sync::Arc, time::Duration};
use tempfile::tempdir;
use tex_index::*;
#[derive(Debug)]
struct Fake;
impl CommandRunner for Fake {
    fn run(&self, _: &Path, args: &[&OsStr], _: Duration) -> Result<CommandResult, TexIndexError> {
        let all = args.first() == Some(&OsStr::new("--all"));
        Ok(CommandResult::new(
            Some(0),
            true,
            if all {
                b"/one/x.sty\n/two/x.sty\n".to_vec()
            } else {
                b"/one/x.sty\n".to_vec()
            },
            Vec::new(),
        ))
    }
}
#[test]
fn resolver_uses_kpsewhich_and_rejects_unsafe_names() {
    let d = tempdir().unwrap();
    std::fs::write(d.path().join("kpsewhich"), b"x").unwrap();
    let c = TexEnvironmentConfig::new_2026(d.path().to_owned()).unwrap();
    let r = KpathseaResolver::new(c, Arc::new(Fake));
    assert_eq!(
        r.resolve("x.sty").unwrap().unwrap().physical_path(),
        Path::new("/one/x.sty")
    );
    assert_eq!(r.resolve_all("x.sty").unwrap().len(), 2);
    for value in ["", "-all", "x\ny", "x\0y"] {
        assert!(r.resolve(value).is_err());
    }
}

#[test]
fn configuration_validation() {
    let directory = tempdir().unwrap();
    assert!(
        TexEnvironmentConfig::new(directory.path().to_owned(), 0, Duration::from_secs(1)).is_err()
    );
    assert!(TexEnvironmentConfig::new(directory.path().to_owned(), 2026, Duration::ZERO).is_err());
    assert!(TexEnvironmentConfig::new_2026(directory.path().join("missing")).is_err());
}

#[cfg(unix)]
#[test]
fn process_runner_success_failure_timeout_and_missing() {
    let runner = ProcessCommandRunner;
    assert!(
        runner
            .run(Path::new("/bin/true"), &[], Duration::from_secs(1))
            .unwrap()
            .success()
    );
    assert!(
        !runner
            .run(Path::new("/bin/false"), &[], Duration::from_secs(1))
            .unwrap()
            .success()
    );
    assert!(matches!(
        runner.run(
            Path::new("/bin/sleep"),
            &[OsStr::new("2")],
            Duration::from_millis(10)
        ),
        Err(TexIndexError::CommandTimedOut { .. })
    ));
    assert!(
        runner
            .run(
                Path::new("/definitely/missing/latex-core-tool"),
                &[],
                Duration::from_secs(1)
            )
            .is_err()
    );
}
