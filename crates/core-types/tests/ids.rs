#![allow(clippy::unwrap_used, reason = "test fixtures use known-valid values")]

use core_types::*;
use std::str::FromStr;
use uuid::Uuid;

#[test]
fn uuid_ids_round_trip_and_are_distinct_types() {
    let id = WorkspaceId::new();
    assert_eq!(WorkspaceId::from_str(&id.to_string()).unwrap(), id);
    assert_eq!(
        serde_json::from_str::<WorkspaceId>(&serde_json::to_string(&id).unwrap()).unwrap(),
        id
    );
    let uuid = Uuid::new_v4();
    let user = UserId::from_uuid(uuid);
    let workspace = WorkspaceId::from_uuid(uuid);
    assert_eq!(user.as_uuid(), workspace.as_uuid());
}

#[test]
fn extension_ids_validate() {
    for value in [
        "vit.spell-check",
        "latex.ai-reader",
        "research.citation-helper",
    ] {
        assert!(ExtensionId::parse(value).is_ok());
    }
    for value in [
        "Spell.Check",
        ".spell",
        "vit.",
        "vit..spell",
        "vit/spell",
        "vit_spell",
        "vit. spell",
        "vit.-spell",
        "vit.spell-",
    ] {
        assert!(ExtensionId::parse(value).is_err(), "{value}");
    }
    let id = ExtensionId::parse("vit.spell-check").unwrap();
    assert_eq!(
        serde_json::from_str::<ExtensionId>(&serde_json::to_string(&id).unwrap()).unwrap(),
        id
    );
}

#[test]
fn validated_compile_identifiers() {
    for value in ["request:ABC_1.2-3", "x"] {
        assert!(IdempotencyKey::parse(value).is_ok());
    }
    for value in ["", "has space", "a/b", "a\\b", "é"] {
        assert!(IdempotencyKey::parse(value).is_err());
    }
    assert!(TexEnvironmentId::parse("texlive-2026-sha256-abc123+full@1").is_ok());
    assert!(TexEnvironmentId::parse("tex/live").is_err());
    assert!(LatexmkProfileId::parse("restricted-v2").is_ok());
    for value in ["Restricted-v2", "bad:profile", "bad/profile", ""] {
        assert!(LatexmkProfileId::parse(value).is_err());
    }
}

#[test]
fn workspace_version_never_wraps() {
    assert_eq!(WorkspaceVersion::initial().get(), 0);
    assert_eq!(WorkspaceVersion::new(4).checked_next().unwrap().get(), 5);
    assert!(WorkspaceVersion::new(u64::MAX).checked_next().is_err());
}
