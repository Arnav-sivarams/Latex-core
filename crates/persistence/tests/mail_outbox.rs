#![cfg(feature = "database-tests")]
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "isolated PostgreSQL integration fixture"
)]

use persistence::{
    AppRepository, Database, DatabaseConfig, GlobalRole, MailOutboxConfig, MailOutboxRepository,
    MailSecretCipher, credentials,
};
use sqlx::PgPool;
use std::{env, time::Duration};
use uuid::Uuid;

#[tokio::test]
async fn encrypted_outbox_send_retry_failure_and_expiry_contract() {
    let url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
    let database = Database::connect(
        DatabaseConfig::new(&url, 1, 5, Duration::from_secs(5)).expect("test config"),
    )
    .await
    .expect("database connects");
    database.migrate().await.expect("migrations apply");
    let pool = PgPool::connect(&url).await.expect("test pool connects");
    let key = MailSecretCipher::from_base64("CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk=")
        .expect("test key");
    let config =
        MailOutboxConfig::new(key, Duration::from_secs(72 * 60 * 60)).expect("mail config");
    let plain_repo = AppRepository::new(database.clone());
    let actor_email = format!("mail-admin-{}@example.test", Uuid::new_v4());
    let actor = plain_repo
        .create_account(
            &actor_email,
            &credentials::hash_password("correct horse battery staple").unwrap(),
        )
        .await
        .unwrap();
    let app = AppRepository::new(database.clone()).with_mail(config.clone());
    let outbox = MailOutboxRepository::new(database.clone(), config, 5);

    let (sent_id, sent_password) = create_delivery(&app, &pool, actor.user_id, "sent").await;
    prioritize(&pool, sent_id).await;
    let delivery = outbox.claim_batch(1).await.unwrap().pop().unwrap();
    assert_eq!(delivery.id, sent_id);
    assert_eq!(delivery.temporary_password, sent_password);
    outbox.mark_sent(&delivery).await.unwrap();
    let sent: (String, bool, bool) = sqlx::query_as(
        "SELECT status,secret_ciphertext IS NULL,secret_nonce IS NULL FROM latex_core.email_outbox WHERE id=$1",
    )
    .bind(sent_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(sent, ("SENT".to_owned(), true, true));

    let (failed_id, _) = create_delivery(&app, &pool, actor.user_id, "failed").await;
    prioritize(&pool, failed_id).await;
    let first = outbox.claim_batch(1).await.unwrap().pop().unwrap();
    outbox
        .mark_failed(&first, true, "smtp_transient")
        .await
        .unwrap();
    let pending: String =
        sqlx::query_scalar("SELECT status FROM latex_core.email_outbox WHERE id=$1")
            .bind(failed_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(pending, "PENDING");
    prioritize(&pool, failed_id).await;
    let second = outbox.claim_batch(1).await.unwrap().pop().unwrap();
    outbox
        .mark_failed(&second, false, "invalid_recipient")
        .await
        .unwrap();
    outbox.retry(failed_id, actor.user_id).await.unwrap();
    let retried: String =
        sqlx::query_scalar("SELECT status FROM latex_core.email_outbox WHERE id=$1")
            .bind(failed_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(retried, "PENDING");

    let (expired_id, _) = create_delivery(&app, &pool, actor.user_id, "expired").await;
    sqlx::query(
        "UPDATE latex_core.email_outbox SET expires_at=statement_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(expired_id)
    .execute(&pool)
    .await
    .unwrap();
    outbox.expire().await.unwrap();
    let expired: (String, bool, bool) = sqlx::query_as(
        "SELECT status,secret_ciphertext IS NULL,secret_nonce IS NULL FROM latex_core.email_outbox WHERE id=$1",
    )
    .bind(expired_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(expired, ("EXPIRED".to_owned(), true, true));
    assert!(outbox.retry(expired_id, actor.user_id).await.is_err());

    pool.close().await;
    database.close().await;
}

async fn create_delivery(
    app: &AppRepository,
    pool: &PgPool,
    actor: core_types::UserId,
    prefix: &str,
) -> (Uuid, String) {
    let email = format!("{prefix}-{}@example.test", Uuid::new_v4());
    let password = credentials::temporary_password();
    let hash = credentials::hash_temporary_password(&password).unwrap();
    let (user, queued) = app
        .create_v2_temporary_account(actor, &email, &hash, GlobalRole::Writer, &password)
        .await
        .unwrap();
    assert!(queued);
    let id = sqlx::query_scalar(
        "SELECT id FROM latex_core.email_outbox WHERE account_user_id=$1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user.user_id.as_uuid())
    .fetch_one(pool)
    .await
    .unwrap();
    (id, password)
}

async fn prioritize(pool: &PgPool, id: Uuid) {
    sqlx::query(
        "UPDATE latex_core.email_outbox SET next_attempt_at='2000-01-01 UTC'::timestamptz WHERE id=$1",
    )
    .bind(id)
    .execute(pool)
    .await
    .unwrap();
}
