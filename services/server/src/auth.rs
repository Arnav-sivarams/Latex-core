#![allow(
    dead_code,
    reason = "the API and admin binaries use complementary auth helpers"
)]

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
    persistence::credentials::validate_password(value).map_err(|_| ())
}

pub fn hash_password(value: &str) -> Result<String, ()> {
    persistence::credentials::hash_password(value).map_err(|_| ())
}

pub fn verify_password(value: &str, hash: &str) -> Result<bool, ()> {
    persistence::credentials::verify_password(value, hash).map_err(|_| ())
}

pub fn temporary_password() -> String {
    persistence::credentials::temporary_password()
}

pub fn hash_temporary_password(value: &str) -> Result<String, ()> {
    persistence::credentials::hash_temporary_password(value).map_err(|_| ())
}
