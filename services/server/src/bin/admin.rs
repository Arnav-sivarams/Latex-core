//! Narrow server-owner account provisioning command. Never expose this over HTTP.
#![forbid(unsafe_code)]

#[path = "../auth.rs"]
mod auth;

use persistence::{AppError, AppRepository, Database, DatabaseConfig};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    if args.next().as_deref() != Some("user") {
        return usage();
    }
    let command = args.next().ok_or("missing user command")?;
    let database =
        Database::connect(DatabaseConfig::development(required("DATABASE_URL")?)?).await?;
    database.migrate().await?;
    let repo = AppRepository::new(database);
    if command == "list" {
        for user in repo.list_users().await? {
            println!(
                "{}\t{}",
                user.email,
                if user.enabled { "enabled" } else { "disabled" }
            );
        }
        return Ok(());
    }
    let email =
        auth::normalized_email(&option(&mut args, "--email")?).map_err(|()| "invalid email")?;
    let password = option(&mut args, "--password").ok();
    match command.as_str() {
        "create" => {
            let password = password.unwrap_or_else(auth::temporary_password);
            let hash = auth::hash_password(&password)
                .map_err(|()| "password must be 12-256 characters")?;
            match repo.create_account(&email, &hash).await {
                Ok(_) => println!("User created\nEmail: {email}\nTemporary password: {password}"),
                Err(AppError::Conflict) => return Err("account already exists".into()),
                Err(error) => return Err(error.into()),
            }
        }
        "disable" => {
            repo.set_user_enabled(&email, false).await?;
            println!("User disabled: {email}");
        }
        "enable" => {
            repo.set_user_enabled(&email, true).await?;
            println!("User enabled: {email}");
        }
        "reset-password" => {
            let password = password.unwrap_or_else(auth::temporary_password);
            let hash = auth::hash_password(&password)
                .map_err(|()| "password must be 12-256 characters")?;
            repo.reset_password(&email, &hash).await?;
            println!("Password reset\nEmail: {email}\nTemporary password: {password}");
        }
        _ => return usage(),
    }
    Ok(())
}

fn option(
    args: &mut impl Iterator<Item = String>,
    name: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    match args.next().as_deref() {
        Some(value) if value == name => args
            .next()
            .ok_or_else(|| format!("missing value for {name}").into()),
        _ => Err(format!("expected {name}").into()),
    }
}
fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    env::var(name).map_err(|_| format!("required environment variable {name} is missing").into())
}
fn usage<T>() -> Result<T, Box<dyn std::error::Error>> {
    Err("usage: latex-core-admin user <create|list|disable|enable|reset-password> --email EMAIL [--password PASSWORD]".into())
}
