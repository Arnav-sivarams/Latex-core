#![allow(
    dead_code,
    reason = "the API and admin binaries use complementary auth helpers"
)]

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use rand::RngCore;

pub fn normalized_email(value: &str) -> Result<String, ()> {
    let email = value.trim().to_ascii_lowercase();
    if email.len() >= 3
        && email.len() <= 320
        && email.contains('@')
        && !email.chars().any(char::is_whitespace)
    {
        Ok(email)
    } else {
        Err(())
    }
}

pub fn validate_password(value: &str) -> Result<(), ()> {
    if (12..=256).contains(&value.len()) {
        Ok(())
    } else {
        Err(())
    }
}

pub fn hash_password(value: &str) -> Result<String, ()> {
    validate_password(value)?;
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| ())
}

pub fn verify_password(value: &str, hash: &str) -> Result<bool, ()> {
    let parsed = PasswordHash::new(hash).map_err(|_| ())?;
    Ok(Argon2::default()
        .verify_password(value.as_bytes(), &parsed)
        .is_ok())
}

pub fn temporary_password() -> String {
    let mut bytes = [0_u8; 24];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}
