#![allow(clippy::unwrap_used, reason = "controlled fixtures")]
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tempfile::TempDir;
use tex_index::*;

#[derive(Debug)]
struct Fake {
    root: PathBuf,
}
struct Wrong(Fake);
impl CommandRunner for Wrong {
    fn run(
        &self,
        program: &Path,
        args: &[&OsStr],
        timeout: Duration,
    ) -> Result<CommandResult, TexIndexError> {
        let mut result = self.0.run(program, args, timeout)?;
        if program.file_name() == Some(OsStr::new("tlmgr")) {
            result = CommandResult::new(Some(0), true, b"TeX Live 2025".to_vec(), Vec::new());
        }
        Ok(result)
    }
}
impl CommandRunner for Fake {
    fn run(
        &self,
        program: &Path,
        args: &[&OsStr],
        _: Duration,
    ) -> Result<CommandResult, TexIndexError> {
        let name = program.file_name().unwrap().to_string_lossy();
        let arg = args.first().and_then(|v| v.to_str()).unwrap_or("");
        let out = if name == "kpsewhich" {
            match arg {
                "--version" => "kpathsea TeX Live 2026".into(),
                "-var-value=TEXMFROOT" => self.root.display().to_string(),
                "-var-value=SELFAUTODIR" => {
                    self.root.join("bin/test-platform").display().to_string()
                }
                logical => {
                    let p = self.root.join("config").join(logical);
                    if p.is_file() {
                        p.display().to_string()
                    } else {
                        String::new()
                    }
                }
            }
        } else if name == "tlmgr" {
            "tlmgr revision 1 (TeX Live 2026)".into()
        } else {
            format!("{name} version 1")
        };
        Ok(CommandResult::new(
            Some(0),
            true,
            out.into_bytes(),
            Vec::new(),
        ))
    }
}
fn fixture(parent: &Path, name: &str) -> PathBuf {
    let root = parent.join(name);
    let bin = root.join("bin/test-platform");
    fs::create_dir_all(&bin).unwrap();
    for tool in TexToolKind::ALL {
        fs::write(
            bin.join(tool.basename()),
            format!("binary-{}", tool.basename()),
        )
        .unwrap();
    }
    let files = [
        ("tex/latex/base/article.cls", b"class".as_slice()),
        ("tex/latex/amsmath/amsmath.sty", b"ams"),
        ("tex/latex/pgf/tikz.sty", b"tikz"),
        ("tex/latex/shadow/tikz.sty", b"shadow"),
        ("tex/latex/IEEEtran/IEEEtran.cls", b"ieee"),
        ("bibtex/bst/base/plain.bst", b"plain"),
        ("tex/latex/biblatex/numeric.bbx", b"bbx"),
        ("tex/latex/biblatex/numeric.cbx", b"cbx"),
        ("fonts/opentype/example/TestFont.otf", b"font"),
    ];
    for (p, b) in files {
        let path = root.join(p);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b).unwrap();
    }
    fs::create_dir_all(root.join("tlpkg")).unwrap();
    fs::write(root.join("tlpkg/texlive.tlpdb"),"name base\ncategory Package\nrevision 1\nrunfiles\n tex/latex/base/article.cls\n bibtex/bst/base/plain.bst\n\nname amsmath\ncategory Package\nrevision 2\nrunfiles\n tex/latex/amsmath/amsmath.sty\n\nname pgf\ncategory Package\nrevision 3\nrunfiles\n tex/latex/pgf/tikz.sty\n\nname shadow\ncategory Package\nrevision 3\nrunfiles\n tex/latex/shadow/tikz.sty\n\nname IEEEtran\ncategory Package\nrevision 4\nrunfiles\n tex/latex/IEEEtran/IEEEtran.cls\n\nname biblatex\ncategory Package\nrevision 5\nrunfiles\n tex/latex/biblatex/numeric.bbx\n tex/latex/biblatex/numeric.cbx\n\nname fonts\ncategory Package\nrevision 6\nrunfiles\n fonts/opentype/example/TestFont.otf\n").unwrap();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::write(root.join("config/texmf.cnf"), b"config").unwrap();
    root
}
fn build_result(root: &Path) -> Result<TexEnvironmentIndexV1, TexIndexError> {
    let config = TexEnvironmentConfig::new_2026(root.join("bin/test-platform")).unwrap();
    TexIndexBuilder::new(
        config,
        Arc::new(Fake {
            root: root.to_owned(),
        }),
    )
    .build()
}
fn build(root: &Path) -> TexEnvironmentIndexV1 {
    build_result(root).unwrap()
}
#[test]
fn deterministic_portable_queries_and_roundtrip() {
    let d = TempDir::new().unwrap();
    let a = fixture(d.path(), "install-a");
    let b = fixture(d.path(), "some path with spaces/install-b");
    let database = b.join("tlpkg/texlive.tlpdb");
    let original = fs::read_to_string(&database).unwrap();
    let mut records: Vec<&str> = original.trim_end().split("\n\n").collect();
    records.reverse();
    fs::write(&database, format!("{}\n", records.join("\n\n"))).unwrap();
    let ia = build(&a);
    let ib = build(&b);
    assert_eq!(ia.environment_id().unwrap(), ib.environment_id().unwrap());
    assert!(
        ia.package_exists("amsmath")
            && ia.style_exists("amsmath")
            && ia.style_exists("tikz")
            && ia.class_exists("article")
            && ia.class_exists("IEEEtran")
            && ia.bibliography_style_exists("plain")
            && ia.biblatex_style_exists("numeric")
            && ia.font_exists("TestFont.otf")
    );
    assert_eq!(ia.find_files_by_basename("tikz.sty").len(), 2);
    assert_eq!(ia.owners_of_basename("tikz.sty"), ["pgf", "shadow"]);
    let bytes = ia.canonical_json_bytes().unwrap();
    let loaded = TexEnvironmentIndexV1::from_json_bytes(&bytes).unwrap();
    assert_eq!(
        ia.environment_id().unwrap(),
        loaded.environment_id().unwrap()
    );
    assert_eq!(bytes, loaded.canonical_json_bytes().unwrap());
}
#[test]
fn content_tool_config_and_revision_invalidate_identity() {
    let d = TempDir::new().unwrap();
    let root = fixture(d.path(), "tree");
    let baseline = build(&root).environment_id().unwrap();
    for (path, data) in [
        ("tex/latex/amsmath/amsmath.sty", b"changed".as_slice()),
        ("tex/latex/base/article.cls", b"changed"),
        ("bin/test-platform/pdflatex", b"changed"),
        ("config/texmf.cnf", b"changed"),
    ] {
        let full = root.join(path);
        let old = fs::read(&full).unwrap();
        fs::write(&full, data).unwrap();
        assert_ne!(baseline, build(&root).environment_id().unwrap());
        fs::write(full, old).unwrap();
    }
    let db = root.join("tlpkg/texlive.tlpdb");
    let old = fs::read_to_string(&db).unwrap();
    fs::write(&db, old.replace("revision 2", "revision 22")).unwrap();
    assert_ne!(baseline, build(&root).environment_id().unwrap());
}
#[test]
fn release_and_schema_are_rejected() {
    let d = TempDir::new().unwrap();
    let root = fixture(d.path(), "tree");
    let c = TexEnvironmentConfig::new_2026(root.join("bin/test-platform")).unwrap();
    assert!(matches!(
        TexIndexBuilder::new(c, Arc::new(Wrong(Fake { root: root.clone() }))).build(),
        Err(TexIndexError::WrongTexLiveRelease { .. })
    ));
    let bytes = build(&root).canonical_json_bytes().unwrap();
    let changed = String::from_utf8(bytes).unwrap().replacen(
        "\"schema_version\":1",
        "\"schema_version\":2",
        1,
    );
    assert!(TexEnvironmentIndexV1::from_json_bytes(changed.as_bytes()).is_err());
}
#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected() {
    use std::os::unix::fs::symlink;
    let d = TempDir::new().unwrap();
    let root = fixture(d.path(), "tree");
    let outside = d.path().join("outside");
    fs::write(&outside, b"secret").unwrap();
    let file = root.join("tex/latex/amsmath/amsmath.sty");
    fs::remove_file(&file).unwrap();
    symlink(outside, file).unwrap();
    assert!(matches!(
        build_result(&root),
        Err(TexIndexError::StorageEntryEscape(_))
    ));
}

#[cfg(unix)]
#[test]
fn cli_builds_and_writes_canonical_index() {
    use std::{os::unix::fs::PermissionsExt, process::Command};
    let directory = TempDir::new().unwrap();
    let root = fixture(directory.path(), "cli tree");
    let bin = root.join("bin/test-platform");
    for kind in TexToolKind::ALL {
        let body = if kind == TexToolKind::Kpsewhich {
            format!(
                "#!/bin/sh\ncase \"$1\" in\n  --version) echo 'kpathsea TeX Live 2026';;\n  -var-value=TEXMFROOT) printf '%s\\n' '{}';;\n  -var-value=SELFAUTODIR) printf '%s\\n' '{}';;\n  texmf.cnf) printf '%s\\n' '{}';;\n  *) exit 1;;\nesac\n",
                root.display(),
                bin.display(),
                root.join("config/texmf.cnf").display()
            )
        } else if kind == TexToolKind::Tlmgr {
            "#!/bin/sh\necho 'tlmgr revision 1 (TeX Live 2026)'\n".to_owned()
        } else {
            format!("#!/bin/sh\necho '{} version 1'\n", kind.basename())
        };
        let path = bin.join(kind.basename());
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let output = directory.path().join("index.json");
    let result = Command::new(env!("CARGO_BIN_EXE_tex-index"))
        .args(["build", "--bin-dir"])
        .arg(&bin)
        .arg("--output")
        .arg(&output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("environment_id=texlive-2026-sha256-"));
    assert!(TexEnvironmentIndexV1::from_json_bytes(&fs::read(output).unwrap()).is_ok());
}
