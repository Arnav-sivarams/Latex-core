#![cfg(feature = "texlive-tests")]
#![allow(clippy::unwrap_used, reason = "explicit integration test")]
use std::{env, path::PathBuf, sync::Arc};
use tex_index::*;
#[test]
fn indexes_configured_tex_live_2026() {
    let bin = PathBuf::from(
        env::var_os("TEXLIVE_BIN_DIR")
            .expect("TEXLIVE_BIN_DIR is required when texlive-tests is enabled"),
    );
    let index = TexIndexBuilder::new(
        TexEnvironmentConfig::new_2026(bin.clone()).unwrap(),
        Arc::new(ProcessCommandRunner),
    )
    .build()
    .unwrap();
    assert_eq!(index.release().year(), 2026);
    assert_eq!(index.tools().len(), 10);
    assert!(!index.packages().is_empty());
    let resolver = KpathseaResolver::new(
        TexEnvironmentConfig::new_2026(bin).unwrap(),
        Arc::new(ProcessCommandRunner),
    );
    for name in ["article.cls", "amsmath.sty", "plain.bst"] {
        assert!(
            resolver.resolve(name).unwrap().is_some()
                || !index.find_files_by_basename(name).is_empty()
        );
    }
    let id = index.environment_id().unwrap();
    assert!(id.as_str().starts_with("texlive-2026-sha256-"));
    let loaded =
        TexEnvironmentIndexV1::from_json_bytes(&index.canonical_json_bytes().unwrap()).unwrap();
    assert_eq!(id, loaded.environment_id().unwrap());
}
