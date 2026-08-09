#![allow(clippy::unwrap_used, reason = "test fixtures use known-valid values")]

use core_types::*;
use std::str::FromStr;

#[test]
fn sha256_known_vector_and_parsing() {
    let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
    let hash = BlobHash::digest(b"abc");
    assert_eq!(hash.to_string(), expected);
    assert_eq!(hash.to_hex(), expected);
    assert_eq!(BlobHash::from_str(expected).unwrap(), hash);
    assert!(BlobHash::from_str(&expected.to_uppercase()).is_err());
    assert!(BlobHash::from_str("abc").is_err());
    assert!(BlobHash::from_str(&"a".repeat(65)).is_err());
    assert!(BlobHash::from_str(&format!("{}g", "a".repeat(63))).is_err());
    assert_eq!(
        serde_json::from_str::<BlobHash>(&serde_json::to_string(&hash).unwrap()).unwrap(),
        hash
    );
    assert_eq!(hash.as_bytes().len(), 32);
}
