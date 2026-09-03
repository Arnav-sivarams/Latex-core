//! Provider-neutral SMTP configuration and temporary-credential delivery.

use async_trait::async_trait;
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor, message::Mailbox,
    transport::smtp::authentication::Credentials,
};
use persistence::{
    ClaimedCredentialEmail, MailOutboxConfig, MailOutboxRepository, MailSecretCipher,
};
use queue::WorkerShutdown;
use std::{env, fmt, sync::Arc, time::Duration};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SmtpSecurity {
    StartTls,
    Tls,
}

#[derive(Clone)]
pub struct MailSettings {
    pub enabled: bool,
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
    from_email: String,
    from_name: String,
    security: SmtpSecurity,
    public_base_url: String,
    outbox: Option<MailOutboxConfig>,
    batch_size: i64,
    max_attempts: u32,
}

impl fmt::Debug for MailSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MailSettings")
            .field("enabled", &self.enabled)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username_configured", &self.username.is_some())
            .field("password", &"[REDACTED]")
            .field("from_email", &self.from_email)
            .field("from_name", &self.from_name)
            .field("security", &self.security)
            .field("public_base_url", &self.public_base_url)
            .finish_non_exhaustive()
    }
}

impl MailSettings {
    pub fn from_env() -> Result<Self, MailConfigError> {
        let enabled = env_bool("LATEX_CORE_MAIL_ENABLED", false)?;
        if !enabled {
            return Ok(Self {
                enabled,
                host: String::new(),
                port: 0,
                username: None,
                password: None,
                from_email: String::new(),
                from_name: String::new(),
                security: SmtpSecurity::StartTls,
                public_base_url: String::new(),
                outbox: None,
                batch_size: 20,
                max_attempts: 5,
            });
        }
        let host = required("LATEX_CORE_SMTP_HOST")?;
        if host.trim().is_empty() || host.chars().any(char::is_whitespace) {
            return Err(MailConfigError::Invalid("LATEX_CORE_SMTP_HOST"));
        }
        let port = parse_number::<u16>("LATEX_CORE_SMTP_PORT", &required("LATEX_CORE_SMTP_PORT")?)?;
        if port == 0 {
            return Err(MailConfigError::Invalid("LATEX_CORE_SMTP_PORT"));
        }
        let username = optional("LATEX_CORE_SMTP_USERNAME");
        let password = optional("LATEX_CORE_SMTP_PASSWORD");
        if username.is_some() != password.is_some() {
            return Err(MailConfigError::CredentialsPair);
        }
        let from_email = required("LATEX_CORE_SMTP_FROM_EMAIL")?;
        from_email
            .parse::<lettre::Address>()
            .map_err(|_| MailConfigError::Invalid("LATEX_CORE_SMTP_FROM_EMAIL"))?;
        let from_name = required("LATEX_CORE_SMTP_FROM_NAME")?;
        if from_name.trim().is_empty() {
            return Err(MailConfigError::Invalid("LATEX_CORE_SMTP_FROM_NAME"));
        }
        let security = match required("LATEX_CORE_SMTP_SECURITY")?
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "starttls" => SmtpSecurity::StartTls,
            "tls" => SmtpSecurity::Tls,
            _ => return Err(MailConfigError::Invalid("LATEX_CORE_SMTP_SECURITY")),
        };
        let public_base_url = required("LATEX_CORE_PUBLIC_BASE_URL")?;
        if !(public_base_url.starts_with("https://") || public_base_url.starts_with("http://")) {
            return Err(MailConfigError::Invalid("LATEX_CORE_PUBLIC_BASE_URL"));
        }
        let cipher = MailSecretCipher::from_base64(&required("LATEX_CORE_MAIL_SECRET_KEY")?)
            .map_err(|_| MailConfigError::Invalid("LATEX_CORE_MAIL_SECRET_KEY"))?;
        let lifetime_hours = env_number("LATEX_CORE_MAIL_SECRET_LIFETIME_HOURS", 72_u64)?;
        if lifetime_hours == 0 || lifetime_hours > 168 {
            return Err(MailConfigError::Invalid(
                "LATEX_CORE_MAIL_SECRET_LIFETIME_HOURS",
            ));
        }
        let batch_size = env_number("LATEX_CORE_MAIL_BATCH_SIZE", 20_i64)?;
        if !(1..=100).contains(&batch_size) {
            return Err(MailConfigError::Invalid("LATEX_CORE_MAIL_BATCH_SIZE"));
        }
        let max_attempts = env_number("LATEX_CORE_MAIL_MAX_ATTEMPTS", 5_u32)?;
        if !(1..=20).contains(&max_attempts) {
            return Err(MailConfigError::Invalid("LATEX_CORE_MAIL_MAX_ATTEMPTS"));
        }
        let outbox = MailOutboxConfig::new(cipher, Duration::from_secs(lifetime_hours * 3600))
            .map_err(|_| MailConfigError::Invalid("LATEX_CORE_MAIL_SECRET_LIFETIME_HOURS"))?;
        Ok(Self {
            enabled,
            host,
            port,
            username,
            password,
            from_email,
            from_name,
            security,
            public_base_url: public_base_url.trim_end_matches('/').to_owned(),
            outbox: Some(outbox),
            batch_size,
            max_attempts,
        })
    }

    pub fn outbox_config(&self) -> Option<MailOutboxConfig> {
        self.outbox.clone()
    }

    pub fn repository(&self, database: persistence::Database) -> Option<MailOutboxRepository> {
        self.outbox
            .clone()
            .map(|config| MailOutboxRepository::new(database, config, self.max_attempts))
    }
}

#[derive(Debug, Error)]
pub enum MailConfigError {
    #[error("required mail environment variable {0} is missing")]
    Missing(&'static str),
    #[error("mail environment variable {0} is malformed")]
    Invalid(&'static str),
    #[error("LATEX_CORE_SMTP_USERNAME and LATEX_CORE_SMTP_PASSWORD must be set together")]
    CredentialsPair,
    #[error("SMTP transport configuration failed")]
    Transport(#[source] lettre::transport::smtp::Error),
    #[error("SMTP sender configuration failed")]
    Sender(#[source] lettre::address::AddressError),
}

fn required(name: &'static str) -> Result<String, MailConfigError> {
    optional(name).ok_or(MailConfigError::Missing(name))
}

fn optional(name: &'static str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn env_bool(name: &'static str, default: bool) -> Result<bool, MailConfigError> {
    match env::var(name) {
        Err(_) => Ok(default),
        Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Ok(true),
            "false" | "0" | "no" => Ok(false),
            _ => Err(MailConfigError::Invalid(name)),
        },
    }
}

fn env_number<T>(name: &'static str, default: T) -> Result<T, MailConfigError>
where
    T: std::str::FromStr,
{
    match env::var(name) {
        Ok(value) => parse_number(name, &value),
        Err(_) => Ok(default),
    }
}

fn parse_number<T>(name: &'static str, value: &str) -> Result<T, MailConfigError>
where
    T: std::str::FromStr,
{
    value.parse().map_err(|_| MailConfigError::Invalid(name))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialMessage {
    pub recipient_email: String,
    pub subject: String,
    pub body: String,
}

impl CredentialMessage {
    fn new(delivery: &ClaimedCredentialEmail, public_base_url: &str) -> Self {
        let role = if delivery.credential_role == "student" {
            "Writer/Student"
        } else {
            "Mentor"
        };
        Self {
            recipient_email: delivery.recipient_email.clone(),
            subject: "Your LaTeX Core account".to_owned(),
            body: format!(
                "LaTeX Core\n\nAccount email: {}\nRole: {}\nTemporary password: {}\nLogin: {}/login\n\nYou will be asked to choose a new password when you first sign in.\n",
                delivery.recipient_email, role, delivery.temporary_password, public_base_url
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MailFailure {
    Transient,
    Permanent,
}

#[async_trait]
pub trait MailTransport: Send + Sync {
    async fn send(&self, message: &CredentialMessage) -> Result<(), MailFailure>;
}

pub struct SmtpMailTransport {
    sender: Mailbox,
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl fmt::Debug for SmtpMailTransport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SmtpMailTransport")
            .field("sender", &self.sender)
            .field("transport", &"[SMTP transport]")
            .finish()
    }
}

impl SmtpMailTransport {
    pub fn new(settings: &MailSettings) -> Result<Self, MailConfigError> {
        let mut builder = match settings.security {
            SmtpSecurity::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&settings.host)
                    .map_err(MailConfigError::Transport)?
            }
            SmtpSecurity::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&settings.host)
                .map_err(MailConfigError::Transport)?,
        }
        .port(settings.port);
        if let (Some(username), Some(password)) = (&settings.username, &settings.password) {
            builder = builder.credentials(Credentials::new(username.clone(), password.clone()));
        }
        let sender = Mailbox::new(
            Some(settings.from_name.clone()),
            settings
                .from_email
                .parse()
                .map_err(MailConfigError::Sender)?,
        );
        Ok(Self {
            sender,
            transport: builder.build(),
        })
    }
}

#[async_trait]
impl MailTransport for SmtpMailTransport {
    async fn send(&self, message: &CredentialMessage) -> Result<(), MailFailure> {
        let email = Message::builder()
            .from(self.sender.clone())
            .to(message
                .recipient_email
                .parse()
                .map_err(|_| MailFailure::Permanent)?)
            .subject(&message.subject)
            .body(message.body.clone())
            .map_err(|_| MailFailure::Permanent)?;
        self.transport
            .send(email)
            .await
            .map(|_| ())
            .map_err(|error| {
                if error.is_transient() {
                    MailFailure::Transient
                } else {
                    MailFailure::Permanent
                }
            })
    }
}

pub async fn run_mail_worker(
    repository: MailOutboxRepository,
    transport: Arc<dyn MailTransport>,
    settings: MailSettings,
    shutdown: WorkerShutdown,
) -> Result<(), persistence::MailOutboxError> {
    let recovered = repository.recover_stale_claims().await?;
    tracing::info!(recovered, "recovered stale email outbox claims");
    while !shutdown.requested() {
        let expired = repository.expire().await?;
        if expired > 0 {
            tracing::info!(expired, "expired credential email payloads");
        }
        let deliveries = repository.claim_batch(settings.batch_size).await?;
        if deliveries.is_empty() {
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        }
        for delivery in deliveries {
            let message = CredentialMessage::new(&delivery, &settings.public_base_url);
            match transport.send(&message).await {
                Ok(()) => {
                    repository.mark_sent(&delivery).await?;
                    tracing::info!(delivery_id=%delivery.id, attempt=delivery.attempt, "credential email sent");
                }
                Err(failure) => {
                    let transient = failure == MailFailure::Transient;
                    let category = if transient {
                        "smtp_transient"
                    } else {
                        "smtp_permanent"
                    };
                    repository
                        .mark_failed(&delivery, transient, category)
                        .await?;
                    tracing::warn!(delivery_id=%delivery.id, attempt=delivery.attempt, error_category=category, "credential email delivery failed");
                }
            }
        }
    }
    Ok(())
}

pub async fn run_disabled_mail_maintenance(
    database: persistence::Database,
    shutdown: WorkerShutdown,
) -> Result<(), persistence::MailOutboxError> {
    let recovered = persistence::recover_stale_credential_email_claims(&database).await?;
    tracing::info!(recovered, "recovered stale email outbox claims");
    while !shutdown.requested() {
        let expired = persistence::expire_credential_email_payloads(&database).await?;
        if expired > 0 {
            tracing::info!(
                expired,
                "expired credential email payloads while mail disabled"
            );
        }
        for _ in 0..60 {
            if shutdown.requested() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Mutex;

    #[derive(Debug, Default)]
    struct FakeTransport(Mutex<Vec<CredentialMessage>>);

    #[async_trait]
    impl MailTransport for FakeTransport {
        async fn send(&self, message: &CredentialMessage) -> Result<(), MailFailure> {
            self.0.lock().await.push(message.clone());
            Ok(())
        }
    }

    #[tokio::test]
    async fn fake_transport_captures_safe_plain_text_message() {
        let delivery = ClaimedCredentialEmail {
            id: uuid::Uuid::new_v4(),
            recipient_email: "student@example.edu".to_owned(),
            account_user_id: None,
            credential_role: "student".to_owned(),
            temporary_password: "Abc2Def3".to_owned(),
            attempt: 1,
        };
        let message = CredentialMessage::new(&delivery, "https://latex.example.edu");
        let fake = FakeTransport::default();
        fake.send(&message).await.expect("fake sends");
        let sent = fake.0.lock().await;
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].subject, "Your LaTeX Core account");
        assert!(sent[0].body.contains("Writer/Student"));
        assert!(sent[0].body.contains("https://latex.example.edu/login"));
        assert!(sent[0].body.contains("choose a new password"));
        assert!(!sent[0].body.contains(&delivery.id.to_string()));
    }
}
