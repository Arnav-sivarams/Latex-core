#![allow(clippy::unwrap_used, reason = "test fixtures use known-valid values")]

use core_types::*;
use std::str::FromStr;

fn material(seed: &[u8]) -> CompileKeyMaterialV1 {
    CompileKeyMaterialV1::new(
        seed_hash(seed),
        TexEngine::PdfLatex,
        TexEnvironmentId::parse("texlive-2026-v1").unwrap(),
        LatexmkProfileId::parse("default-v1").unwrap(),
        ShellPolicy::Safe,
        true,
    )
}
fn seed_hash(seed: &[u8]) -> SnapshotId {
    BlobHash::digest(seed).to_string().parse().unwrap()
}

#[test]
fn enums_have_locked_strings() {
    for (engine, text) in [
        (TexEngine::PdfLatex, "pdflatex"),
        (TexEngine::LuaLatex, "lualatex"),
        (TexEngine::XeLatex, "xelatex"),
    ] {
        assert_eq!(engine.to_string(), text);
        assert_eq!(TexEngine::from_str(text).unwrap(), engine);
        assert_eq!(
            serde_json::to_string(&engine).unwrap(),
            format!("\"{text}\"")
        );
    }
    assert!(TexEngine::from_str("tectonic").is_err());
    assert_eq!(
        serde_json::to_string(&ShellPolicy::Restricted).unwrap(),
        "\"restricted\""
    );
    assert_eq!(
        serde_json::to_string(&CostClass::Heavy).unwrap(),
        "\"heavy\""
    );
    assert_eq!(
        serde_json::to_string(&JobState::Cancelled).unwrap(),
        "\"cancelled\""
    );
}

#[test]
fn request_round_trip() {
    let workspace_id = WorkspaceId::new();
    let snapshot_id = seed_hash(b"snap");
    let idempotency_key = IdempotencyKey::parse("click:1").unwrap();
    let request = CompileRequestV1::new(
        workspace_id,
        snapshot_id,
        TexEngine::LuaLatex,
        ShellPolicy::Safe,
        true,
        idempotency_key.clone(),
    );

    assert_eq!(request.schema_version(), CompileRequestV1::SCHEMA_VERSION);

    let json = serde_json::to_string(&request).unwrap();
    let restored = serde_json::from_str::<CompileRequestV1>(&json).unwrap();
    assert_eq!(restored.workspace_id(), workspace_id);
    assert_eq!(restored.snapshot_id(), snapshot_id);
    assert_eq!(restored.engine(), TexEngine::LuaLatex);
    assert_eq!(restored.shell_policy(), ShellPolicy::Safe);
    assert!(restored.synctex());
    assert_eq!(restored.idempotency_key(), &idempotency_key);
    assert_eq!(restored, request);
}

#[test]
fn request_rejects_unsupported_or_missing_schema_versions() {
    let request = CompileRequestV1::new(
        WorkspaceId::new(),
        seed_hash(b"schema"),
        TexEngine::PdfLatex,
        ShellPolicy::Restricted,
        false,
        IdempotencyKey::parse("schema-check").unwrap(),
    );
    let mut value = serde_json::to_value(&request).unwrap();

    value["schema_version"] = serde_json::json!(999);
    assert!(serde_json::from_value::<CompileRequestV1>(value.clone()).is_err());

    value["schema_version"] = serde_json::json!(0);
    assert!(serde_json::from_value::<CompileRequestV1>(value.clone()).is_err());

    value.as_object_mut().unwrap().remove("schema_version");
    assert!(serde_json::from_value::<CompileRequestV1>(value).is_err());

    let valid_json = serde_json::to_string(&request).unwrap();
    assert!(serde_json::from_str::<CompileRequestV1>(&valid_json).is_ok());
}

#[test]
fn compile_key_is_deterministic_and_all_material_invalidates() {
    let base = material(b"a");
    let key = base.compile_key().unwrap();
    assert_eq!(key, material(b"a").compile_key().unwrap());
    let cases = [
        CompileKeyMaterialV1::new(
            seed_hash(b"b"),
            TexEngine::PdfLatex,
            TexEnvironmentId::parse("texlive-2026-v1").unwrap(),
            LatexmkProfileId::parse("default-v1").unwrap(),
            ShellPolicy::Safe,
            true,
        ),
        CompileKeyMaterialV1::new(
            seed_hash(b"a"),
            TexEngine::XeLatex,
            TexEnvironmentId::parse("texlive-2026-v1").unwrap(),
            LatexmkProfileId::parse("default-v1").unwrap(),
            ShellPolicy::Safe,
            true,
        ),
        CompileKeyMaterialV1::new(
            seed_hash(b"a"),
            TexEngine::PdfLatex,
            TexEnvironmentId::parse("texlive-2027-v1").unwrap(),
            LatexmkProfileId::parse("default-v1").unwrap(),
            ShellPolicy::Safe,
            true,
        ),
        CompileKeyMaterialV1::new(
            seed_hash(b"a"),
            TexEngine::PdfLatex,
            TexEnvironmentId::parse("texlive-2026-v1").unwrap(),
            LatexmkProfileId::parse("safe-v1").unwrap(),
            ShellPolicy::Safe,
            true,
        ),
        CompileKeyMaterialV1::new(
            seed_hash(b"a"),
            TexEngine::PdfLatex,
            TexEnvironmentId::parse("texlive-2026-v1").unwrap(),
            LatexmkProfileId::parse("default-v1").unwrap(),
            ShellPolicy::Compatibility,
            true,
        ),
        CompileKeyMaterialV1::new(
            seed_hash(b"a"),
            TexEngine::PdfLatex,
            TexEnvironmentId::parse("texlive-2026-v1").unwrap(),
            LatexmkProfileId::parse("default-v1").unwrap(),
            ShellPolicy::Safe,
            false,
        ),
    ];
    for changed in cases {
        assert_ne!(key, changed.compile_key().unwrap());
    }
    let restored: CompileKeyMaterialV1 =
        serde_json::from_str(&serde_json::to_string(&base).unwrap()).unwrap();
    assert_eq!(key, restored.compile_key().unwrap());
}
