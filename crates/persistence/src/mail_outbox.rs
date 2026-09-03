//! Encrypted temporary-credential outbox and bounded delivery state transitions.

use crate::Database;
use aes_gcm::{
    Aes256Gcm, KeyInit, Nonce,
    aead::{
        Aead, Payload,
        rand_core::{OsRng, RngCore},
    },
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use core_types::UserId;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Row, postgres::PgRow};
use std::{fmt, sync::Arc, time::Duration};
use thiserror::Error;
use uuid::Uuid;

const NONCE_LENGTH: usize = 12;

#[derive(Debug, Error)]
pub enum MailOutboxError {
    #[error("mail secret key must be base64-encoded random 32-byte material")]
    InvalidKey,
    #[error("mail secret encryption failed")]
    Encryption,
    #[error("mail secret decryption failed")]
    Decryption,
    #[error("mail outbox database operation failed")]
    Database(#[source] sqlx::Error),
    #[error("email delivery record not found")]
    NotFound,
    #[error("email delivery payload has expired or is unavailable")]
    PayloadUnavailable,
}

#[derive(Clone)]
pub struct MailSecretCipher(Arc<Aes256Gcm>);

impl fmt::Debug for MailSecretCipher {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MailSecretCipher([REDACTED])")
    }
}

impl MailSecretCipher {
    pub fn from_base64(value: &str) -> Result<Self, MailOutboxError> {
        let decoded = STANDARD
            .decode(value.trim())
            .map_err(|_| MailOutboxError::InvalidKey)?;
        let key: [u8; 32] = decoded
            .try_into()
            .map_err(|_| MailOutboxError::InvalidKey)?;
        Ok(Self(Arc::new(Aes256Gcm::new((&key).into()))))
    }

    fn encrypt(
        &self,
        id: Uuid,
        recipient: &str,
        role: &str,
        plaintext: &str,
    ) -> Result<(Vec<u8>, Vec<u8>), MailOutboxError> {
        let mut nonce = [0_u8; NONCE_LENGTH];
        OsRng.fill_bytes(&mut nonce);
        let ciphertext = self
            .0
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: associated_data(id, recipient, role).as_bytes(),
                },
            )
            .map_err(|_| MailOutboxError::Encryption)?;
        Ok((ciphertext, nonce.to_vec()))
    }

    fn decrypt(
        &self,
        id: Uuid,
        recipient: &str,
        role: &str,
        ciphertext: &[u8],
        nonce: &[u8],
    ) -> Result<String, MailOutboxError> {
        if nonce.len() != NONCE_LENGTH {
            return Err(MailOutboxError::Decryption);
        }
        let plaintext = self
            .0
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: associated_data(id, recipient, role).as_bytes(),
                },
            )
            .map_err(|_| MailOutboxError::Decryption)?;
        String::from_utf8(plaintext).map_err(|_| MailOutboxError::Decryption)
    }
}

fn associated_data(id: Uuid, recipient: &str, role: &str) -> String {
    format!("latex-core:temporary-credential:{id}:{recipient}:{role}")
}

#[derive(Clone, Debug)]
pub struct MailOutboxConfig {
    cipher: MailSecretCipher,
    lifetime_seconds: i64,
}

impl MailOutboxConfig {
    pub fn new(cipher: MailSecretCipher, lifetime: Duration) -> Result<Self, MailOutboxError> {
        let lifetime_seconds =
            i64::try_from(lifetime.as_secs()).map_err(|_| MailOutboxError::PayloadUnavailable)?;
        if lifetime_seconds <= 0 {
            return Err(MailOutboxError::PayloadUnavailable);
        }
        Ok(Self {
            cipher,
            lifetime_seconds,
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ClaimedCredentialEmail {
    pub id: Uuid,
    pub recipient_email: String,
    pub account_user_id: Option<UserId>,
    pub credential_role: String,
    #[serde(skip)]
    pub temporary_password: String,
    pub attempt: u32,
}

impl fmt::Debug for ClaimedCredentialEmail {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimedCredentialEmail")
            .field("id", &self.id)
            .field("recipient_email", &self.recipient_email)
            .field("account_user_id", &self.account_user_id)
            .field("credential_role", &self.credential_role)
            .field("temporary_password", &"[REDACTED]")
            .field("attempt", &self.attempt)
            .finish()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MailOutboxSummary {
    pub pending: i64,
    pub sent: i64,
    pub failed: i64,
}

#[derive(Clone, Debug)]
pub struct MailOutboxRepository {
    database: Database,
    config: MailOutboxConfig,
    max_attempts: u32,
}

impl MailOutboxRepository {
    #[must_use]
    pub const fn new(database: Database, config: MailOutboxConfig, max_attempts: u32) -> Self {
        Self {
            database,
            config,
            max_attempts,
        }
    }

    pub async fn recover_stale_claims(&self) -> Result<u64, MailOutboxError> {
        let result = sqlx::query(
            "UPDATE latex_core.email_outbox SET status='PENDING',claimed_at=NULL,next_attempt_at=statement_timestamp() \
             WHERE status='SENDING' AND claimed_at < statement_timestamp() - interval '10 minutes'",
        )
        .execute(self.database.pool())
        .await
        .map_err(MailOutboxError::Database)?;
        Ok(result.rows_affected())
    }

    pub async fn expire(&self) -> Result<u64, MailOutboxError> {
        let result = sqlx::query(
            "UPDATE latex_core.email_outbox SET status='EXPIRED',secret_ciphertext=NULL,secret_nonce=NULL,claimed_at=NULL,last_error='credential delivery expired' \
             WHERE status IN ('PENDING','SENDING','FAILED') AND expires_at <= statement_timestamp()",
        )
        .execute(self.database.pool())
        .await
        .map_err(MailOutboxError::Database)?;
        Ok(result.rows_affected())
    }

    pub async fn claim_batch(
        &self,
        limit: i64,
    ) -> Result<Vec<ClaimedCredentialEmail>, MailOutboxError> {
        let rows = sqlx::query(
            r"WITH candidates AS (
                 SELECT id FROM latex_core.email_outbox
                 WHERE status='PENDING' AND next_attempt_at <= statement_timestamp()
                   AND expires_at > statement_timestamp() AND secret_ciphertext IS NOT NULL
                 ORDER BY next_attempt_at,created_at,id FOR UPDATE SKIP LOCKED LIMIT $1
               )
               UPDATE latex_core.email_outbox outbox
               SET status='SENDING',attempts=outbox.attempts+1,claimed_at=statement_timestamp(),last_error=NULL
               FROM candidates WHERE outbox.id=candidates.id
               RETURNING outbox.id,outbox.recipient_email,outbox.account_user_id,outbox.credential_role,
                         outbox.secret_ciphertext,outbox.secret_nonce,outbox.attempts",
        )
        .bind(limit.clamp(1, 100))
        .fetch_all(self.database.pool())
        .await
        .map_err(MailOutboxError::Database)?;
        rows.iter().map(|row| self.decode_claim(row)).collect()
    }

    fn decode_claim(&self, row: &PgRow) -> Result<ClaimedCredentialEmail, MailOutboxError> {
        let id: Uuid = row.try_get("id").map_err(MailOutboxError::Database)?;
        let recipient_email: String = row
            .try_get("recipient_email")
            .map_err(MailOutboxError::Database)?;
        let credential_role: String = row
            .try_get("credential_role")
            .map_err(MailOutboxError::Database)?;
        let ciphertext: Vec<u8> = row
            .try_get("secret_ciphertext")
            .map_err(MailOutboxError::Database)?;
        let nonce: Vec<u8> = row
            .try_get("secret_nonce")
            .map_err(MailOutboxError::Database)?;
        let attempts: i32 = row.try_get("attempts").map_err(MailOutboxError::Database)?;
        Ok(ClaimedCredentialEmail {
            id,
            temporary_password: self.config.cipher.decrypt(
                id,
                &recipient_email,
                &credential_role,
                &ciphertext,
                &nonce,
            )?,
            recipient_email,
            account_user_id: row
                .try_get::<Option<Uuid>, _>("account_user_id")
                .map_err(MailOutboxError::Database)?
                .map(UserId::from_uuid),
            credential_role,
            attempt: u32::try_from(attempts).unwrap_or(u32::MAX),
        })
    }

    pub async fn mark_sent(
        &self,
        delivery: &ClaimedCredentialEmail,
    ) -> Result<(), MailOutboxError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(MailOutboxError::Database)?;
        sqlx::query(
            "UPDATE latex_core.email_outbox SET status='SENT',sent_at=statement_timestamp(),claimed_at=NULL,last_error=NULL,secret_ciphertext=NULL,secret_nonce=NULL WHERE id=$1 AND status='SENDING'",
        )
        .bind(delivery.id)
        .execute(&mut *tx)
        .await
        .map_err(MailOutboxError::Database)?;
        audit_delivery(&mut tx, delivery, "account.credential_email.sent", None).await?;
        tx.commit().await.map_err(MailOutboxError::Database)
    }

    pub async fn mark_failed(
        &self,
        delivery: &ClaimedCredentialEmail,
        transient: bool,
        category: &str,
    ) -> Result<(), MailOutboxError> {
        let retry = transient && delivery.attempt < self.max_attempts;
        let delay_seconds = retry_delay_seconds(delivery.attempt);
        let status = if retry { "PENDING" } else { "FAILED" };
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(MailOutboxError::Database)?;
        sqlx::query(
            "UPDATE latex_core.email_outbox SET status=$2,claimed_at=NULL,last_error=$3,next_attempt_at=statement_timestamp()+make_interval(secs=>$4) WHERE id=$1 AND status='SENDING'",
        )
        .bind(delivery.id)
        .bind(status)
        .bind(category)
        .bind(delay_seconds)
        .execute(&mut *tx)
        .await
        .map_err(MailOutboxError::Database)?;
        if !retry {
            audit_delivery(
                &mut tx,
                delivery,
                "account.credential_email.failed",
                Some(category),
            )
            .await?;
        }
        tx.commit().await.map_err(MailOutboxError::Database)
    }

    pub async fn retry(&self, id: Uuid, actor: UserId) -> Result<(), MailOutboxError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(MailOutboxError::Database)?;
        let user_id: Option<Uuid> = sqlx::query_scalar(
            "UPDATE latex_core.email_outbox SET status='PENDING',next_attempt_at=statement_timestamp(),last_error=NULL \
             WHERE id=$1 AND status='FAILED' AND expires_at>statement_timestamp() AND secret_ciphertext IS NOT NULL RETURNING account_user_id",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(MailOutboxError::Database)?
        .flatten();
        if user_id.is_none() {
            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM latex_core.email_outbox WHERE id=$1)",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(MailOutboxError::Database)?;
            return Err(if exists {
                MailOutboxError::PayloadUnavailable
            } else {
                MailOutboxError::NotFound
            });
        }
        sqlx::query(
            "INSERT INTO latex_core.audit_events (id,actor_user_id,event_type,resource_type,resource_id,metadata) \
             VALUES ($1,$2,'account.credential_email.retried','email_outbox',$3,jsonb_build_object('user_id',$4::text))",
        )
        .bind(Uuid::new_v4())
        .bind(actor.as_uuid())
        .bind(id)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(MailOutboxError::Database)?;
        tx.commit().await.map_err(MailOutboxError::Database)
    }

    pub async fn summary(&self) -> Result<MailOutboxSummary, MailOutboxError> {
        let row = sqlx::query(
            "SELECT count(*) FILTER (WHERE status IN ('PENDING','SENDING')) AS pending,count(*) FILTER (WHERE status='SENT') AS sent,count(*) FILTER (WHERE status='FAILED') AS failed FROM latex_core.email_outbox",
        )
        .fetch_one(self.database.pool())
        .await
        .map_err(MailOutboxError::Database)?;
        Ok(MailOutboxSummary {
            pending: row.try_get("pending").map_err(MailOutboxError::Database)?,
            sent: row.try_get("sent").map_err(MailOutboxError::Database)?,
            failed: row.try_get("failed").map_err(MailOutboxError::Database)?,
        })
    }
}

/// Recovers abandoned delivery claims without requiring access to the encryption key.
pub async fn recover_stale_credential_email_claims(
    database: &Database,
) -> Result<u64, MailOutboxError> {
    let result = sqlx::query(
        "UPDATE latex_core.email_outbox SET status='PENDING',claimed_at=NULL,next_attempt_at=statement_timestamp() \
         WHERE status='SENDING' AND claimed_at < statement_timestamp() - interval '10 minutes'",
    )
    .execute(database.pool())
    .await
    .map_err(MailOutboxError::Database)?;
    Ok(result.rows_affected())
}

/// Erases expired encrypted payloads without requiring access to the encryption key.
pub async fn expire_credential_email_payloads(database: &Database) -> Result<u64, MailOutboxError> {
    let result = sqlx::query(
        "UPDATE latex_core.email_outbox SET status='EXPIRED',secret_ciphertext=NULL,secret_nonce=NULL,claimed_at=NULL,last_error='credential delivery expired' \
         WHERE status IN ('PENDING','SENDING','FAILED') AND expires_at <= statement_timestamp()",
    )
    .execute(database.pool())
    .await
    .map_err(MailOutboxError::Database)?;
    Ok(result.rows_affected())
}

pub(crate) async fn enqueue_temporary_credential_tx(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    config: &MailOutboxConfig,
    actor: UserId,
    account_user_id: UserId,
    recipient: &str,
    credential_role: &str,
    temporary_password: &str,
) -> Result<Uuid, MailOutboxError> {
    let id = Uuid::new_v4();
    let (ciphertext, nonce) =
        config
            .cipher
            .encrypt(id, recipient, credential_role, temporary_password)?;
    sqlx::query(
        "INSERT INTO latex_core.email_outbox \
         (id,recipient_email,email_type,account_user_id,credential_role,secret_ciphertext,secret_nonce,expires_at) \
         VALUES ($1,$2,'TEMPORARY_CREDENTIAL',$3,$4,$5,$6,statement_timestamp()+make_interval(secs=>$7))",
    )
    .bind(id)
    .bind(recipient)
    .bind(account_user_id.as_uuid())
    .bind(credential_role)
    .bind(ciphertext)
    .bind(nonce)
    .bind(config.lifetime_seconds)
    .execute(&mut **tx)
    .await
    .map_err(MailOutboxError::Database)?;
    sqlx::query(
        "INSERT INTO latex_core.audit_events (id,actor_user_id,event_type,resource_type,resource_id,metadata) \
         VALUES ($1,$2,'account.credential_email.queued','email_outbox',$3,jsonb_build_object('user_id',$4::text))",
    )
    .bind(Uuid::new_v4())
    .bind(actor.as_uuid())
    .bind(id)
    .bind(account_user_id.as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(MailOutboxError::Database)?;
    Ok(id)
}

async fn audit_delivery(
    tx: &mut sqlx::Transaction<'_, Postgres>,
    delivery: &ClaimedCredentialEmail,
    event: &str,
    category: Option<&str>,
) -> Result<(), MailOutboxError> {
    sqlx::query(
        "INSERT INTO latex_core.audit_events (id,actor_user_id,event_type,resource_type,resource_id,metadata) \
         VALUES ($1,NULL,$2,'email_outbox',$3,jsonb_build_object('user_id',$4::text,'attempt',$5,'error_category',$6::text))",
    )
    .bind(Uuid::new_v4())
    .bind(event)
    .bind(delivery.id)
    .bind(delivery.account_user_id.map(|user| *user.as_uuid()))
    .bind(i64::from(delivery.attempt))
    .bind(category)
    .execute(&mut **tx)
    .await
    .map_err(MailOutboxError::Database)?;
    Ok(())
}

const fn retry_delay_seconds(attempt: u32) -> i32 {
    match attempt {
        0 | 1 => 60,
        2 => 5 * 60,
        3 => 15 * 60,
        _ => 60 * 60,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_secret_round_trips_without_containing_plaintext() {
        let key = STANDARD.encode([7_u8; 32]);
        let cipher = MailSecretCipher::from_base64(&key).expect("valid key");
        let id = Uuid::new_v4();
        let secret = "A2bcDef3";
        let (ciphertext, nonce) = cipher
            .encrypt(id, "writer@example.edu", "student", secret)
            .expect("encrypts");
        assert!(
            !ciphertext
                .windows(secret.len())
                .any(|bytes| bytes == secret.as_bytes())
        );
        assert_eq!(
            cipher
                .decrypt(id, "writer@example.edu", "student", &ciphertext, &nonce)
                .expect("decrypts"),
            secret
        );
    }

    #[test]
    fn invalid_or_wrong_length_keys_are_rejected() {
        assert!(MailSecretCipher::from_base64("not base64").is_err());
        assert!(MailSecretCipher::from_base64(&STANDARD.encode([0_u8; 16])).is_err());
    }

    #[test]
    fn retry_schedule_is_bounded() {
        assert_eq!(retry_delay_seconds(1), 60);
        assert_eq!(retry_delay_seconds(2), 300);
        assert_eq!(retry_delay_seconds(3), 900);
        assert_eq!(retry_delay_seconds(20), 3600);
    }
}
