//! Narrow server-owner account provisioning command. Never expose this over HTTP.
#![forbid(unsafe_code)]

#[path = "../archive.rs"]
mod archive;
#[path = "../auth.rs"]
mod auth;

use blob_store::{BlobStore, FsBlobStore, FsBlobStoreConfig};
use persistence::{
    AppError, AppRepository, AppTemplateFileRecord, Database, DatabaseConfig, V2Repository,
};
use std::env;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let group = args.next().ok_or("missing command")?;
    let command = args.next().ok_or("missing subcommand")?;
    let database =
        Database::connect(DatabaseConfig::development(required("DATABASE_URL")?)?).await?;
    database.migrate().await?;
    let v2 = V2Repository::new(database.clone());
    let repo = AppRepository::new(database);
    if group == "template" {
        return templates(repo, args, &command).await;
    }
    if group != "user" {
        return usage();
    }
    if command == "list" {
        for user in v2.list_v2_users().await? {
            println!("{}", user_list_line(&user));
        }
        return Ok(());
    }
    let first = args.next().ok_or("missing email")?;
    let email = if first == "--email" {
        args.next().ok_or("missing email")?
    } else {
        first
    };
    let email = auth::normalized_email(&email).map_err(|()| "invalid email")?;
    if command == "set-type" {
        let account_type = args.next().ok_or("missing account type")?;
        if !matches!(account_type.as_str(), "student" | "professor" | "admin") {
            return Err("account type must be student, professor, or admin".into());
        }
        repo.set_user_account_type(&email, &account_type).await?;
        println!("Institutional account type set: {email} → {account_type}");
        return Ok(());
    }
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

#[allow(
    clippy::too_many_lines,
    reason = "the small administration CLI keeps commands together"
)]
async fn templates(
    repo: AppRepository,
    mut args: impl Iterator<Item = String>,
    command: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        "list" => {
            println!("Templates");
            for template in repo.list_templates().await? {
                println!(
                    "{}\t{}",
                    template.name,
                    template.main_file.unwrap_or_else(|| "-".into())
                );
            }
        }
        "remove" => {
            let name = args.next().ok_or("missing template name")?;
            repo.delete_template_by_name(&name).await?;
            println!("Template removed: {name}");
        }
        "set-audience" => {
            let name = args.next().ok_or("missing template name")?;
            let audiences: Vec<String> = args.collect();
            let values: Vec<&str> = audiences.iter().map(String::as_str).collect();
            repo.set_template_audiences(&name, &values).await?;
            println!("Template audience updated: {name}");
        }
        "grant-user" => {
            let name = args.next().ok_or("missing template name")?;
            let target_email = auth::normalized_email(&args.next().ok_or("missing user email")?)
                .map_err(|()| "invalid user email")?;
            if args.next().as_deref() != Some("--by") {
                return Err("grant-user requires --by ADMIN_EMAIL".into());
            }
            let actor_email = auth::normalized_email(&args.next().ok_or("missing admin email")?)
                .map_err(|()| "invalid admin email")?;
            let target = repo
                .user_by_email(&target_email)
                .await?
                .ok_or("target account not found")?;
            let actor = repo
                .user_by_email(&actor_email)
                .await?
                .ok_or("granting account not found")?;
            if actor.account_type != "admin" {
                return Err("granting account must have institutional type admin".into());
            }
            repo.grant_template_to_user(&name, target.user_id, actor.user_id)
                .await?;
            println!("Template grant added: {name} → {target_email}");
        }
        "add" => {
            let zip = args.next().ok_or("missing ZIP path")?;
            let mut name = None;
            let mut description = None;
            let mut requested_main = None;
            while let Some(flag) = args.next() {
                let value = args
                    .next()
                    .ok_or_else(|| format!("missing value for {flag}"))?;
                match flag.as_str() {
                    "--name" => name = Some(value),
                    "--description" => description = Some(value),
                    "--main" => requested_main = Some(value),
                    _ => return Err(format!("unknown option {flag}").into()),
                }
            }
            let name = name.ok_or("template name is required")?;
            if name.trim().is_empty() || name.len() > 200 {
                return Err("invalid template name".into());
            }
            let input = tokio::fs::read(zip).await?;
            let imported = archive::read_archive(&input).map_err(|error| error.to_string())?;
            let main = match requested_main {
                Some(path) => {
                    let path =
                        core_types::LogicalPath::parse(&path).map_err(|_| "invalid main path")?;
                    if !imported.files.iter().any(|file| file.path == path) {
                        return Err("template main file is absent from archive".into());
                    }
                    Some(path)
                }
                None => imported.detected_main.clone(),
            };
            let store = Arc::new(
                FsBlobStore::open(
                    required("BLOB_STORAGE_ROOT")?,
                    FsBlobStoreConfig::development_default(),
                )
                .await?,
            );
            let mut files = Vec::with_capacity(imported.files.len());
            for file in imported.files {
                let stored = store.put(file.bytes).await?;
                files.push(AppTemplateFileRecord {
                    path: file.path.as_str().to_owned(),
                    blob_hash: stored.hash(),
                    size_bytes: stored.size_bytes(),
                });
            }
            let id = uuid::Uuid::new_v4();
            repo.create_template(
                id,
                name.trim(),
                description.as_deref(),
                main.as_ref().map(core_types::LogicalPath::as_str),
                &files,
            )
            .await?;
            println!(
                "Template added\nName: {}\nFiles: {}\nMain: {}",
                name.trim(),
                files.len(),
                main.map_or_else(|| "-".into(), |path| path.to_string())
            );
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
fn user_list_line(user: &persistence::V2User) -> String {
    format!(
        "{}\t{}\tv2={}\tlegacy={}",
        user.email,
        if user.enabled { "enabled" } else { "disabled" },
        user.v2_role.map_or("UNASSIGNED", |role| role.as_str()),
        user.legacy_account_type,
    )
}
fn usage<T>() -> Result<T, Box<dyn std::error::Error>> {
    Err("usage: latex-core-admin user <create|list|disable|enable|reset-password|set-type> EMAIL [--password PASSWORD] | template <add|list|remove|set-audience|grant-user>".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_list_reports_authoritative_v2_and_legacy_roles() {
        let assigned = persistence::V2User {
            user_id: core_types::UserId::new(),
            email: "writer@example.test".into(),
            enabled: true,
            legacy_account_type: "student".into(),
            v2_role: Some(persistence::GlobalRole::Writer),
            migration_state: "ASSIGNED".into(),
            created_at: "now".into(),
        };
        assert_eq!(
            user_list_line(&assigned),
            "writer@example.test\tenabled\tv2=writer\tlegacy=student"
        );
        let unassigned = persistence::V2User {
            v2_role: None,
            migration_state: "UNASSIGNED".into(),
            ..assigned
        };
        assert!(user_list_line(&unassigned).contains("v2=UNASSIGNED\tlegacy=student"));
    }
}
