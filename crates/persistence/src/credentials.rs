//! Password hashing and cryptographically secure temporary credentials.

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{
        SaltString,
        rand_core::{OsRng, RngCore},
    },
};
use thiserror::Error;

const UPPER: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ";
const LOWER: &[u8] = b"abcdefghijkmnopqrstuvwxyz";
const DIGITS: &[u8] = b"23456789";
const EASY: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
pub const TEMPORARY_PASSWORD_LENGTH: usize = 8;

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum CredentialError {
    #[error("password does not meet policy")]
    Policy,
    #[error("password hashing failed")]
    Hash,
    #[error("stored password hash is invalid")]
    InvalidHash,
}

/// Validates a permanent password against the server policy.
///
/// # Errors
///
/// Returns [`CredentialError::Policy`] when the password is outside the accepted length range.
pub fn validate_password(value: &str) -> Result<(), CredentialError> {
    if (12..=256).contains(&value.len()) {
        Ok(())
    } else {
        Err(CredentialError::Policy)
    }
}

/// Hashes a policy-compliant permanent password with Argon2.
///
/// # Errors
///
/// Returns a policy error for an invalid password or a hash error if Argon2 fails.
pub fn hash_password(value: &str) -> Result<String, CredentialError> {
    validate_password(value)?;
    hash_unchecked(value)
}

/// Hashes an exactly eight-character temporary password with Argon2.
///
/// # Errors
///
/// Returns a policy error for a value of any other length or a hash error if Argon2 fails.
pub fn hash_temporary_password(value: &str) -> Result<String, CredentialError> {
    if value.len() != TEMPORARY_PASSWORD_LENGTH {
        return Err(CredentialError::Policy);
    }
    hash_unchecked(value)
}

fn hash_unchecked(value: &str) -> Result<String, CredentialError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(value.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| CredentialError::Hash)
}

/// Verifies a supplied password against a stored Argon2 hash.
///
/// # Errors
///
/// Returns [`CredentialError::InvalidHash`] when the stored hash cannot be parsed.
pub fn verify_password(value: &str, hash: &str) -> Result<bool, CredentialError> {
    let parsed = PasswordHash::new(hash).map_err(|_| CredentialError::InvalidHash)?;
    Ok(Argon2::default()
        .verify_password(value.as_bytes(), &parsed)
        .is_ok())
}

pub fn temporary_password() -> String {
    let mut rng = OsRng;
    let mut password = [0_u8; TEMPORARY_PASSWORD_LENGTH];
    password[0] = UPPER[secure_index(&mut rng, UPPER.len())];
    password[1] = LOWER[secure_index(&mut rng, LOWER.len())];
    password[2] = DIGITS[secure_index(&mut rng, DIGITS.len())];
    for value in &mut password[3..] {
        *value = EASY[secure_index(&mut rng, EASY.len())];
    }
    for index in (1..password.len()).rev() {
        let swap = secure_index(&mut rng, index + 1);
        password.swap(index, swap);
    }
    password.into_iter().map(char::from).collect()
}

fn secure_index(rng: &mut OsRng, upper_bound: usize) -> usize {
    let Ok(bound) = u32::try_from(upper_bound) else {
        unreachable!("credential alphabet fits in u32");
    };
    let rejection_limit = u32::MAX - (u32::MAX % bound);
    loop {
        let candidate = rng.next_u32();
        if candidate < rejection_limit {
            let Ok(index) = usize::try_from(candidate % bound) else {
                unreachable!("u32 index fits usize");
            };
            return index;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temporary_passwords_have_the_required_shape_and_vary() {
        let passwords = (0..128).map(|_| temporary_password()).collect::<Vec<_>>();
        for password in &passwords {
            assert_eq!(password.len(), TEMPORARY_PASSWORD_LENGTH);
            assert!(password.bytes().any(|value| UPPER.contains(&value)));
            assert!(password.bytes().any(|value| LOWER.contains(&value)));
            assert!(password.bytes().any(|value| DIGITS.contains(&value)));
            assert!(password.bytes().all(|value| EASY.contains(&value)));
        }
        assert!(passwords.windows(2).any(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn temporary_password_hashes_are_argon2_and_verifiable() {
        let password = temporary_password();
        let hash = hash_temporary_password(&password).expect("temporary password hashes");
        assert!(hash.starts_with("$argon2"));
        assert_eq!(verify_password(&password, &hash), Ok(true));
        assert!(!hash.contains(&password));
    }
}
