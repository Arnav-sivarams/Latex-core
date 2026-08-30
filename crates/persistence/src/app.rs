//! Release-facing `PostgreSQL` records, deliberately scoped by authenticated owner.
#![allow(
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    reason = "public repository methods share AppError and decoding consumes SQL rows at call sites"
)]

use crate::{
    AccountType, Database, GlobalRole, GroupRoles, OverrideEffect, Permission, PermissionResolver,
    ProjectRoles,
};
use core_types::{ArtifactId, BlobHash, JobId, TenantId, UserId, WorkspaceId};
use serde_json::json;
use sqlx::Row;
use std::str::FromStr;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("record not found")]
    NotFound,
    #[error("record already exists")]
    Conflict,
    #[error("operation is not authorized")]
    Forbidden,
    #[error("member draft is based on a stale canonical file revision")]
    DraftConflict,
    #[error("persistence failed")]
    Database(#[source] sqlx::Error),
    #[error("persistent data is invalid: {message}")]
    Integrity { message: String },
}

#[derive(Clone, Debug)]
pub struct AppUserRecord {
    pub user_id: UserId,
    pub tenant_id: TenantId,
    pub email: String,
    pub password_hash: String,
    pub enabled: bool,
    pub account_type: String,
}
#[derive(Clone, Debug)]
pub struct AppSessionRecord {
    pub user_id: UserId,
    pub tenant_id: TenantId,
    pub email: String,
    pub account_type: String,
    pub global_role: Option<GlobalRole>,
}
#[derive(Clone, Debug)]
pub struct AppProjectRecord {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug)]
pub struct AppJobRecord {
    pub id: JobId,
    pub workspace_id: WorkspaceId,
    pub state: String,
    pub snapshot_id: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    pub last_error: Option<serde_json::Value>,
    pub queue_position: Option<u64>,
    pub jobs_ahead: Option<u64>,
}
#[derive(Clone, Debug)]
pub struct AppArtifactRecord {
    pub id: ArtifactId,
    pub logical_name: String,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
    pub content_type: String,
}
#[derive(Clone, Debug)]
pub struct AppTemplateRecord {
    pub id: uuid::Uuid,
    pub name: String,
    pub description: Option<String>,
    pub main_file: Option<String>,
    pub created_at: String,
}
#[derive(Clone, Debug)]
pub struct AppTemplateFileRecord {
    pub path: String,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
}
#[derive(Clone, Debug)]
pub struct ResearchGroupRecord {
    pub id: uuid::Uuid,
    pub name: String,
    pub owner_user_id: UserId,
    pub workspace_id: WorkspaceId,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug)]
pub struct ResearchGroupMemberRecord {
    pub user_id: UserId,
    pub email: String,
    pub joined_at: String,
}
#[derive(Clone, Debug)]
pub struct AdminUserRecord {
    pub user_id: UserId,
    pub email: String,
    pub enabled: bool,
    pub account_type: String,
    pub created_at: String,
}

#[derive(Clone, Debug)]
pub struct AppRepository {
    pub(crate) database: Database,
}

impl AppRepository {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn create_account(
        &self,
        email: &str,
        password_hash: &str,
    ) -> Result<AppUserRecord, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let tenant = TenantId::new();
        let user = UserId::new();
        sqlx::query("INSERT INTO latex_core.tenants (id) VALUES ($1)")
            .bind(tenant.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        sqlx::query("INSERT INTO latex_core.users (id,tenant_id) VALUES ($1,$2)")
            .bind(user.as_uuid())
            .bind(tenant.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        let insert = sqlx::query("INSERT INTO latex_core.user_credentials (user_id,email,password_hash) VALUES ($1,$2,$3)").bind(user.as_uuid()).bind(email).bind(password_hash).execute(&mut *tx).await;
        match insert {
            Ok(_) => {}
            Err(error) if matches!(error.as_database_error().and_then(sqlx::error::DatabaseError::code), Some(code) if code == "23505") =>
            {
                return Err(AppError::Conflict);
            }
            Err(error) => return Err(AppError::Database(error)),
        }
        tx.commit().await.map_err(AppError::Database)?;
        Ok(AppUserRecord {
            user_id: user,
            tenant_id: tenant,
            email: email.to_owned(),
            password_hash: password_hash.to_owned(),
            enabled: true,
            account_type: "student".to_owned(),
        })
    }

    /// Creates a V2 identity and its authoritative exclusive role in one
    /// transaction. The legacy account type remains the least-privilege
    /// compatibility value and is never used to authorize this identity.
    pub async fn create_v2_account(
        &self,
        email: &str,
        password_hash: &str,
        role: GlobalRole,
    ) -> Result<AppUserRecord, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let tenant = TenantId::new();
        let user = UserId::new();
        sqlx::query("INSERT INTO latex_core.tenants (id) VALUES ($1)")
            .bind(tenant.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        sqlx::query("INSERT INTO latex_core.users (id,tenant_id) VALUES ($1,$2)")
            .bind(user.as_uuid())
            .bind(tenant.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        let insert = sqlx::query(
            "INSERT INTO latex_core.user_credentials (user_id,email,password_hash,account_type) \
             VALUES ($1,$2,$3,'student')",
        )
        .bind(user.as_uuid())
        .bind(email)
        .bind(password_hash)
        .execute(&mut *tx)
        .await;
        match insert {
            Ok(_) => {}
            Err(error) if matches!(error.as_database_error().and_then(sqlx::error::DatabaseError::code), Some(code) if code == "23505") =>
            {
                return Err(AppError::Conflict);
            }
            Err(error) => return Err(AppError::Database(error)),
        }
        sqlx::query("INSERT INTO latex_core.global_user_roles (user_id,role) VALUES ($1,$2)")
            .bind(user.as_uuid())
            .bind(role.as_str())
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(AppUserRecord {
            user_id: user,
            tenant_id: tenant,
            email: email.to_owned(),
            password_hash: password_hash.to_owned(),
            enabled: true,
            account_type: "student".to_owned(),
        })
    }
    pub async fn user_by_email(&self, email: &str) -> Result<Option<AppUserRecord>, AppError> {
        let row = sqlx::query("SELECT u.id,u.tenant_id,c.email,c.password_hash,c.enabled,c.account_type FROM latex_core.user_credentials c JOIN latex_core.users u ON u.id=c.user_id WHERE c.email=$1").bind(email).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?;
        row.map(decode_user).transpose()
    }
    pub async fn create_session(
        &self,
        digest: &str,
        user: UserId,
        expires_seconds: i64,
    ) -> Result<(), AppError> {
        sqlx::query("INSERT INTO latex_core.sessions (token_digest,user_id,expires_at) VALUES ($1,$2,statement_timestamp()+($3::bigint * interval '1 second'))").bind(digest).bind(user.as_uuid()).bind(expires_seconds).execute(self.database.pool()).await.map_err(AppError::Database)?;
        Ok(())
    }
    pub async fn session(&self, digest: &str) -> Result<Option<AppSessionRecord>, AppError> {
        let row=sqlx::query("SELECT u.id,u.tenant_id,c.email,c.account_type,g.role AS global_role FROM latex_core.sessions s JOIN latex_core.users u ON u.id=s.user_id JOIN latex_core.user_credentials c ON c.user_id=u.id LEFT JOIN latex_core.global_user_roles g ON g.user_id=u.id WHERE s.token_digest=$1 AND s.expires_at>statement_timestamp() AND c.enabled=TRUE") .bind(digest).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?;
        row.map(|r| {
            let global_role = r
                .try_get::<Option<String>, _>("global_role")
                .map_err(AppError::Database)?
                .map(|role| role.parse::<GlobalRole>())
                .transpose()
                .map_err(|error| AppError::Integrity {
                    message: error.to_string(),
                })?;
            Ok(AppSessionRecord {
                user_id: UserId::from_uuid(r.try_get("id").map_err(AppError::Database)?),
                tenant_id: TenantId::from_uuid(r.try_get("tenant_id").map_err(AppError::Database)?),
                email: r.try_get("email").map_err(AppError::Database)?,
                account_type: r.try_get("account_type").map_err(AppError::Database)?,
                global_role,
            })
        })
        .transpose()
    }
    pub async fn delete_session(&self, digest: &str) -> Result<(), AppError> {
        sqlx::query("DELETE FROM latex_core.sessions WHERE token_digest=$1")
            .bind(digest)
            .execute(self.database.pool())
            .await
            .map_err(AppError::Database)?;
        Ok(())
    }
    pub async fn list_users(&self) -> Result<Vec<AppUserRecord>, AppError> {
        let rows = sqlx::query("SELECT u.id,u.tenant_id,c.email,c.password_hash,c.enabled,c.account_type FROM latex_core.user_credentials c JOIN latex_core.users u ON u.id=c.user_id ORDER BY c.email").fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_user).collect()
    }
    pub async fn set_user_enabled(&self, email: &str, enabled: bool) -> Result<(), AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let result =
            sqlx::query("UPDATE latex_core.user_credentials SET enabled=$2 WHERE email=$1")
                .bind(email)
                .bind(enabled)
                .execute(&mut *tx)
                .await
                .map_err(AppError::Database)?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        sqlx::query("DELETE FROM latex_core.sessions WHERE user_id=(SELECT user_id FROM latex_core.user_credentials WHERE email=$1)").bind(email).execute(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)
    }
    pub async fn set_user_account_type(
        &self,
        email: &str,
        account_type: &str,
    ) -> Result<(), AppError> {
        if !matches!(account_type, "student" | "professor" | "admin") {
            return Err(AppError::Integrity {
                message: "invalid institutional account type".into(),
            });
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        if account_type == "student" {
            let unsafe_demotion: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.teams t JOIN latex_core.team_members target ON target.team_id=t.id JOIN latex_core.user_credentials target_credentials ON target_credentials.user_id=target.user_id WHERE t.group_type='mentor_group' AND target.group_manager AND target_credentials.email=$1 AND (SELECT count(*) FROM latex_core.team_members manager JOIN latex_core.user_credentials credentials ON credentials.user_id=manager.user_id WHERE manager.team_id=t.id AND manager.group_manager AND credentials.account_type IN ('professor','admin')) <= 1)")
                .bind(email).fetch_one(&mut *tx).await.map_err(AppError::Database)?;
            if unsafe_demotion {
                return Err(AppError::Integrity { message: "assign another professor or administrator mentor-group manager before this account-type change".into() });
            }
        }
        let result =
            sqlx::query("UPDATE latex_core.user_credentials SET account_type=$2 WHERE email=$1")
                .bind(email)
                .bind(account_type)
                .execute(&mut *tx)
                .await
                .map_err(AppError::Database)?;
        if result.rows_affected() == 0 {
            Err(AppError::NotFound)
        } else {
            tx.commit().await.map_err(AppError::Database)
        }
    }
    pub async fn reset_password(&self, email: &str, password_hash: &str) -> Result<(), AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let result =
            sqlx::query("UPDATE latex_core.user_credentials SET password_hash=$2 WHERE email=$1")
                .bind(email)
                .bind(password_hash)
                .execute(&mut *tx)
                .await
                .map_err(AppError::Database)?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        sqlx::query("DELETE FROM latex_core.sessions WHERE user_id=(SELECT user_id FROM latex_core.user_credentials WHERE email=$1)").bind(email).execute(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)
    }
    pub async fn create_project(
        &self,
        workspace: WorkspaceId,
        owner: UserId,
        name: &str,
    ) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO latex_core.projects (workspace_id,owner_user_id,name) VALUES ($1,$2,$3)",
        )
        .bind(workspace.as_uuid())
        .bind(owner.as_uuid())
        .bind(name)
        .execute(self.database.pool())
        .await
        .map_err(map_conflict)?;
        Ok(())
    }
    pub async fn create_research_group(
        &self,
        owner: UserId,
        workspace: WorkspaceId,
        name: &str,
    ) -> Result<ResearchGroupRecord, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let id = uuid::Uuid::new_v4();
        sqlx::query("INSERT INTO latex_core.research_groups (id,name,owner_user_id,workspace_id) VALUES ($1,$2,$3,$4)")
            .bind(id).bind(name).bind(owner.as_uuid()).bind(workspace.as_uuid()).execute(&mut *tx).await.map_err(map_conflict)?;
        sqlx::query(
            "INSERT INTO latex_core.research_group_members (group_id,user_id) VALUES ($1,$2)",
        )
        .bind(id)
        .bind(owner.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        let row = sqlx::query("SELECT id,name,owner_user_id,workspace_id,created_at::text,updated_at::text FROM latex_core.research_groups WHERE id=$1")
            .bind(id).fetch_one(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        decode_research_group(row)
    }
    pub async fn research_groups_for_user(
        &self,
        user: UserId,
    ) -> Result<Vec<ResearchGroupRecord>, AppError> {
        let rows = sqlx::query("SELECT g.id,g.name,g.owner_user_id,g.workspace_id,g.created_at::text,g.updated_at::text FROM latex_core.research_groups g JOIN latex_core.research_group_members m ON m.group_id=g.id WHERE m.user_id=$1 ORDER BY g.updated_at DESC,g.id")
            .bind(user.as_uuid()).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_research_group).collect()
    }
    pub async fn research_group_for_user(
        &self,
        user: UserId,
        id: uuid::Uuid,
    ) -> Result<ResearchGroupRecord, AppError> {
        let row = sqlx::query("SELECT g.id,g.name,g.owner_user_id,g.workspace_id,g.created_at::text,g.updated_at::text FROM latex_core.research_groups g JOIN latex_core.research_group_members m ON m.group_id=g.id WHERE g.id=$1 AND m.user_id=$2")
            .bind(id).bind(user.as_uuid()).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        decode_research_group(row)
    }
    pub async fn research_group_members(
        &self,
        user: UserId,
        id: uuid::Uuid,
    ) -> Result<Vec<ResearchGroupMemberRecord>, AppError> {
        self.research_group_for_user(user, id).await?;
        let rows = sqlx::query("SELECT m.user_id,c.email,m.joined_at::text FROM latex_core.research_group_members m JOIN latex_core.user_credentials c ON c.user_id=m.user_id WHERE m.group_id=$1 ORDER BY c.email")
            .bind(id).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter()
            .map(|row| {
                Ok(ResearchGroupMemberRecord {
                    user_id: UserId::from_uuid(row.try_get("user_id").map_err(AppError::Database)?),
                    email: row.try_get("email").map_err(AppError::Database)?,
                    joined_at: row.try_get("joined_at").map_err(AppError::Database)?,
                })
            })
            .collect()
    }
    pub async fn rename_research_group(
        &self,
        actor: UserId,
        id: uuid::Uuid,
        name: &str,
    ) -> Result<(), AppError> {
        let result = sqlx::query("UPDATE latex_core.research_groups SET name=$3,updated_at=now() WHERE id=$1 AND owner_user_id=$2").bind(id).bind(actor.as_uuid()).bind(name).execute(self.database.pool()).await.map_err(AppError::Database)?;
        if result.rows_affected() == 0 {
            return Err(AppError::Forbidden);
        }
        Ok(())
    }
    pub async fn add_research_group_member(
        &self,
        actor: UserId,
        id: uuid::Uuid,
        target: UserId,
    ) -> Result<(), AppError> {
        let owner: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.research_groups WHERE id=$1 AND owner_user_id=$2)").bind(id).bind(actor.as_uuid()).fetch_one(self.database.pool()).await.map_err(AppError::Database)?;
        if !owner {
            return Err(AppError::Forbidden);
        }
        sqlx::query("INSERT INTO latex_core.research_group_members (group_id,user_id) VALUES ($1,$2) ON CONFLICT DO NOTHING").bind(id).bind(target.as_uuid()).execute(self.database.pool()).await.map_err(AppError::Database)?;
        Ok(())
    }
    pub async fn remove_research_group_member(
        &self,
        actor: UserId,
        id: uuid::Uuid,
        target: UserId,
    ) -> Result<(), AppError> {
        let owner: UserId =
            sqlx::query_scalar("SELECT owner_user_id FROM latex_core.research_groups WHERE id=$1")
                .bind(id)
                .fetch_optional(self.database.pool())
                .await
                .map_err(AppError::Database)?
                .map(UserId::from_uuid)
                .ok_or(AppError::NotFound)?;
        if owner != actor {
            return Err(AppError::Forbidden);
        }
        if owner == target {
            return Err(AppError::Integrity {
                message: "the research group owner cannot be removed".into(),
            });
        }
        let result = sqlx::query(
            "DELETE FROM latex_core.research_group_members WHERE group_id=$1 AND user_id=$2",
        )
        .bind(id)
        .bind(target.as_uuid())
        .execute(self.database.pool())
        .await
        .map_err(AppError::Database)?;
        if result.rows_affected() == 0 {
            Err(AppError::NotFound)
        } else {
            Ok(())
        }
    }
    pub async fn delete_research_group(
        &self,
        actor: UserId,
        id: uuid::Uuid,
    ) -> Result<(), AppError> {
        let owner: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.research_groups WHERE id=$1 AND owner_user_id=$2)").bind(id).bind(actor.as_uuid()).fetch_one(self.database.pool()).await.map_err(AppError::Database)?;
        if !owner {
            return Err(AppError::Forbidden);
        }
        Err(AppError::Integrity {
            message: "research group workspace deletion is not implemented".into(),
        })
    }
    pub async fn admin_users(&self) -> Result<Vec<AdminUserRecord>, AppError> {
        let rows = sqlx::query("SELECT c.user_id,c.email,c.enabled,c.account_type,u.created_at::text FROM latex_core.user_credentials c JOIN latex_core.users u ON u.id=c.user_id ORDER BY c.email").fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter()
            .map(|row| {
                Ok(AdminUserRecord {
                    user_id: UserId::from_uuid(row.try_get("user_id").map_err(AppError::Database)?),
                    email: row.try_get("email").map_err(AppError::Database)?,
                    enabled: row.try_get("enabled").map_err(AppError::Database)?,
                    account_type: row.try_get("account_type").map_err(AppError::Database)?,
                    created_at: row.try_get("created_at").map_err(AppError::Database)?,
                })
            })
            .collect()
    }
    pub async fn delete_user_if_unreferenced(&self, email: &str) -> Result<(), AppError> {
        let mut transaction = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let user_id = sqlx::query_scalar::<_, uuid::Uuid>(
            "DELETE FROM latex_core.user_credentials WHERE email=$1 RETURNING user_id",
        )
        .bind(email)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(AppError::Database)?
        .ok_or_else(|| AppError::Integrity {
            message: "User owns resources and cannot be deleted. Disable the account instead."
                .into(),
        })?;
        let result = sqlx::query("DELETE FROM latex_core.users WHERE id=$1")
            .bind(user_id)
            .execute(&mut *transaction)
            .await
            .map_err(AppError::Database)?;
        if result.rows_affected() != 1 {
            return Err(AppError::Integrity {
                message: "User owns resources and cannot be deleted. Disable the account instead."
                    .into(),
            });
        }
        transaction.commit().await.map_err(AppError::Database)
    }
    pub async fn admin_overview(&self) -> Result<serde_json::Value, AppError> {
        async fn count(pool: &sqlx::PgPool, table: &str) -> Result<i64, AppError> {
            sqlx::query_scalar(&format!("SELECT count(*) FROM latex_core.{table}"))
                .fetch_one(pool)
                .await
                .map_err(AppError::Database)
        }
        let pool = self.database.pool();
        Ok(
            json!({"users": count(pool,"users").await?, "personal_projects": count(pool,"projects").await?, "research_groups": count(pool,"research_groups").await?, "teams": count(pool,"teams").await?, "team_projects": count(pool,"team_projects").await?, "queued_jobs": sqlx::query_scalar::<_,i64>("SELECT count(*) FROM latex_core.compile_jobs WHERE state='queued'").fetch_one(pool).await.map_err(AppError::Database)?, "running_jobs": sqlx::query_scalar::<_,i64>("SELECT count(*) FROM latex_core.compile_jobs WHERE state IN ('claimed','running')").fetch_one(pool).await.map_err(AppError::Database)?}),
        )
    }
    pub async fn admin_teams(&self) -> Result<Vec<serde_json::Value>, AppError> {
        let rows = sqlx::query("SELECT t.id,t.name,c.email AS creator,t.created_at::text,(SELECT count(*) FROM latex_core.team_members m WHERE m.team_id=t.id) AS members,(SELECT count(*) FROM latex_core.team_projects p WHERE p.team_id=t.id) AS projects FROM latex_core.teams t JOIN latex_core.user_credentials c ON c.user_id=t.created_by ORDER BY t.created_at DESC").fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(|r| Ok(json!({"id":r.try_get::<uuid::Uuid,_>("id").map_err(AppError::Database)?.to_string(),"name":r.try_get::<String,_>("name").map_err(AppError::Database)?,"creator":r.try_get::<String,_>("creator").map_err(AppError::Database)?,"created_at":r.try_get::<String,_>("created_at").map_err(AppError::Database)?,"members":r.try_get::<i64,_>("members").map_err(AppError::Database)?,"projects":r.try_get::<i64,_>("projects").map_err(AppError::Database)?}))).collect()
    }
    pub async fn admin_research_groups(&self) -> Result<Vec<serde_json::Value>, AppError> {
        let rows = sqlx::query("SELECT g.id,g.name,g.workspace_id,c.email AS owner,g.created_at::text,(SELECT count(*) FROM latex_core.research_group_members m WHERE m.group_id=g.id) AS members FROM latex_core.research_groups g JOIN latex_core.user_credentials c ON c.user_id=g.owner_user_id ORDER BY g.created_at DESC").fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(|r| Ok(json!({"id":r.try_get::<uuid::Uuid,_>("id").map_err(AppError::Database)?.to_string(),"name":r.try_get::<String,_>("name").map_err(AppError::Database)?,"workspace_id":r.try_get::<uuid::Uuid,_>("workspace_id").map_err(AppError::Database)?.to_string(),"owner":r.try_get::<String,_>("owner").map_err(AppError::Database)?,"created_at":r.try_get::<String,_>("created_at").map_err(AppError::Database)?,"members":r.try_get::<i64,_>("members").map_err(AppError::Database)?}))).collect()
    }
    pub async fn admin_projects(&self) -> Result<Vec<serde_json::Value>, AppError> {
        let rows = sqlx::query("SELECT p.workspace_id,p.name,'Personal' AS kind,c.email AS container,p.updated_at::text FROM latex_core.projects p JOIN latex_core.user_credentials c ON c.user_id=p.owner_user_id UNION ALL SELECT g.workspace_id,g.name,'Research Group',c.email,g.updated_at::text FROM latex_core.research_groups g JOIN latex_core.user_credentials c ON c.user_id=g.owner_user_id UNION ALL SELECT p.workspace_id,p.name,'Team',t.name,p.updated_at::text FROM latex_core.team_projects p JOIN latex_core.teams t ON t.id=p.team_id ORDER BY updated_at DESC").fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(|r| Ok(json!({"workspace_id":r.try_get::<uuid::Uuid,_>("workspace_id").map_err(AppError::Database)?.to_string(),"name":r.try_get::<String,_>("name").map_err(AppError::Database)?,"type":r.try_get::<String,_>("kind").map_err(AppError::Database)?,"container":r.try_get::<String,_>("container").map_err(AppError::Database)?,"updated_at":r.try_get::<String,_>("updated_at").map_err(AppError::Database)?}))).collect()
    }
    pub async fn admin_jobs(&self) -> Result<Vec<serde_json::Value>, AppError> {
        let rows = sqlx::query("SELECT j.id,j.workspace_id,j.state,j.created_at::text,j.started_at::text,j.finished_at::text,c.email FROM latex_core.compile_jobs j JOIN latex_core.user_credentials c ON c.user_id=j.user_id ORDER BY j.created_at DESC LIMIT 100").fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(|r| Ok(json!({"id":r.try_get::<uuid::Uuid,_>("id").map_err(AppError::Database)?.to_string(),"workspace_id":r.try_get::<uuid::Uuid,_>("workspace_id").map_err(AppError::Database)?.to_string(),"state":r.try_get::<String,_>("state").map_err(AppError::Database)?,"requester":r.try_get::<String,_>("email").map_err(AppError::Database)?,"created_at":r.try_get::<String,_>("created_at").map_err(AppError::Database)?,"started_at":r.try_get::<Option<String>,_>("started_at").map_err(AppError::Database)?,"finished_at":r.try_get::<Option<String>,_>("finished_at").map_err(AppError::Database)?}))).collect()
    }
    pub async fn admin_audit(&self) -> Result<Vec<serde_json::Value>, AppError> {
        let rows = sqlx::query("SELECT a.event_type,a.resource_type,a.resource_id,a.created_at::text,c.email FROM latex_core.audit_events a LEFT JOIN latex_core.user_credentials c ON c.user_id=a.actor_user_id ORDER BY a.created_at DESC LIMIT 100").fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(|r| Ok(json!({"event":r.try_get::<String,_>("event_type").map_err(AppError::Database)?,"resource":r.try_get::<String,_>("resource_type").map_err(AppError::Database)?,"resource_id":r.try_get::<Option<uuid::Uuid>,_>("resource_id").map_err(AppError::Database)?.map(|v|v.to_string()),"actor":r.try_get::<Option<String>,_>("email").map_err(AppError::Database)?,"created_at":r.try_get::<String,_>("created_at").map_err(AppError::Database)?}))).collect()
    }
    pub async fn create_template(
        &self,
        id: uuid::Uuid,
        name: &str,
        description: Option<&str>,
        main_file: Option<&str>,
        files: &[AppTemplateFileRecord],
    ) -> Result<(), AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        sqlx::query(
            "INSERT INTO latex_core.templates (id,name,description,main_file) VALUES ($1,$2,$3,$4)",
        )
        .bind(id)
        .bind(name)
        .bind(description)
        .bind(main_file)
        .execute(&mut *tx)
        .await
        .map_err(map_conflict)?;
        for account_type in ["student", "professor", "admin"] {
            sqlx::query("INSERT INTO latex_core.template_account_types (template_id,account_type) VALUES ($1,$2)")
                .bind(id).bind(account_type).execute(&mut *tx).await.map_err(AppError::Database)?;
        }
        for file in files {
            let size = i64::try_from(file.size_bytes).map_err(|_| AppError::Integrity {
                message: "template file size exceeds PostgreSQL BIGINT".into(),
            })?;
            sqlx::query("INSERT INTO latex_core.template_files (template_id,path,blob_hash,size_bytes) VALUES ($1,$2,$3,$4)")
                .bind(id).bind(&file.path).bind(file.blob_hash.to_string()).bind(size).execute(&mut *tx).await.map_err(AppError::Database)?;
        }
        tx.commit().await.map_err(AppError::Database)
    }
    pub async fn list_templates(&self) -> Result<Vec<AppTemplateRecord>, AppError> {
        let rows = sqlx::query("SELECT id,name,description,main_file,created_at::text FROM latex_core.templates ORDER BY name")
            .fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_template).collect()
    }
    pub async fn list_templates_for_user(
        &self,
        user: UserId,
    ) -> Result<Vec<AppTemplateRecord>, AppError> {
        let rows = sqlx::query("SELECT DISTINCT t.id,t.name,t.description,t.main_file,t.created_at::text FROM latex_core.templates t JOIN latex_core.user_credentials c ON c.user_id=$1 LEFT JOIN latex_core.template_account_types a ON a.template_id=t.id AND a.account_type=c.account_type LEFT JOIN latex_core.template_user_grants g ON g.template_id=t.id AND g.user_id=$1 WHERE a.template_id IS NOT NULL OR g.user_id IS NOT NULL ORDER BY t.name")
            .bind(user.as_uuid()).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_template).collect()
    }
    pub async fn assert_template_visible(
        &self,
        user: UserId,
        template: uuid::Uuid,
    ) -> Result<(), AppError> {
        let account: String = sqlx::query_scalar(
            "SELECT account_type FROM latex_core.user_credentials WHERE user_id=$1",
        )
        .bind(user.as_uuid())
        .fetch_optional(self.database.pool())
        .await
        .map_err(AppError::Database)?
        .ok_or(AppError::NotFound)?;
        let overrides = sqlx::query(
            "SELECT permission,effect FROM latex_core.permission_overrides WHERE user_id=$1 AND context_kind='global'",
        )
        .bind(user.as_uuid())
        .fetch_all(self.database.pool())
        .await
        .map_err(AppError::Database)?
        .into_iter()
        .map(|row| {
            let permission: String = row.try_get("permission").map_err(AppError::Database)?;
            let effect: String = row.try_get("effect").map_err(AppError::Database)?;
            let permission = Permission::parse(&permission).ok_or_else(|| AppError::Integrity {
                message: "invalid permission override".into(),
            })?;
            let effect = match effect.as_str() {
                "allow" => OverrideEffect::Allow,
                "deny" => OverrideEffect::Deny,
                _ => return Err(AppError::Integrity { message: "invalid permission override effect".into() }),
            };
            Ok((permission, effect))
        })
        .collect::<Result<Vec<_>, AppError>>()?;
        let resolver = PermissionResolver::new(
            AccountType::parse(&account)?,
            GroupRoles::default(),
            ProjectRoles::default(),
            overrides,
        );
        if !resolver.allows(Permission::TemplateUse) {
            return Err(AppError::Forbidden);
        }
        let visible: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.templates t JOIN latex_core.user_credentials c ON c.user_id=$1 LEFT JOIN latex_core.template_account_types a ON a.template_id=t.id AND a.account_type=c.account_type LEFT JOIN latex_core.template_user_grants g ON g.template_id=t.id AND g.user_id=$1 WHERE t.id=$2 AND (a.template_id IS NOT NULL OR g.user_id IS NOT NULL))")
            .bind(user.as_uuid()).bind(template).fetch_one(self.database.pool()).await.map_err(AppError::Database)?;
        if visible {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }
    pub async fn template(&self, id: uuid::Uuid) -> Result<AppTemplateRecord, AppError> {
        let row = sqlx::query("SELECT id,name,description,main_file,created_at::text FROM latex_core.templates WHERE id=$1")
            .bind(id).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        decode_template(row)
    }
    pub async fn template_files(
        &self,
        id: uuid::Uuid,
    ) -> Result<Vec<AppTemplateFileRecord>, AppError> {
        let rows = sqlx::query("SELECT path,blob_hash,size_bytes FROM latex_core.template_files WHERE template_id=$1 ORDER BY path")
            .bind(id).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_template_file).collect()
    }
    pub async fn delete_template_by_name(&self, name: &str) -> Result<(), AppError> {
        let result = sqlx::query("DELETE FROM latex_core.templates WHERE name=$1")
            .bind(name)
            .execute(self.database.pool())
            .await
            .map_err(AppError::Database)?;
        if result.rows_affected() == 0 {
            Err(AppError::NotFound)
        } else {
            Ok(())
        }
    }
    pub async fn set_template_audiences(
        &self,
        name: &str,
        audiences: &[&str],
    ) -> Result<(), AppError> {
        if audiences.is_empty()
            || audiences
                .iter()
                .any(|value| !matches!(*value, "student" | "professor" | "admin"))
        {
            return Err(AppError::Integrity {
                message: "template audiences must be institutional account types".into(),
            });
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let id: uuid::Uuid =
            sqlx::query_scalar("SELECT id FROM latex_core.templates WHERE name=$1 FOR UPDATE")
                .bind(name)
                .fetch_optional(&mut *tx)
                .await
                .map_err(AppError::Database)?
                .ok_or(AppError::NotFound)?;
        sqlx::query("DELETE FROM latex_core.template_account_types WHERE template_id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        for audience in audiences {
            sqlx::query("INSERT INTO latex_core.template_account_types (template_id,account_type) VALUES ($1,$2)").bind(id).bind(*audience).execute(&mut *tx).await.map_err(AppError::Database)?;
        }
        tx.commit().await.map_err(AppError::Database)
    }
    pub async fn grant_template_to_user(
        &self,
        name: &str,
        user: UserId,
        granted_by: UserId,
    ) -> Result<(), AppError> {
        let template: uuid::Uuid =
            sqlx::query_scalar("SELECT id FROM latex_core.templates WHERE name=$1")
                .bind(name)
                .fetch_optional(self.database.pool())
                .await
                .map_err(AppError::Database)?
                .ok_or(AppError::NotFound)?;
        sqlx::query("INSERT INTO latex_core.template_user_grants (template_id,user_id,granted_by_user_id) VALUES ($1,$2,$3) ON CONFLICT (template_id,user_id) DO NOTHING")
            .bind(template).bind(user.as_uuid()).bind(granted_by.as_uuid()).execute(self.database.pool()).await.map_err(AppError::Database)?;
        Ok(())
    }
    pub async fn list_projects(&self, owner: UserId) -> Result<Vec<AppProjectRecord>, AppError> {
        let rows=sqlx::query("SELECT workspace_id,name,created_at::text,updated_at::text FROM latex_core.projects WHERE owner_user_id=$1 ORDER BY updated_at DESC").bind(owner.as_uuid()).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_project).collect()
    }
    pub async fn project(
        &self,
        owner: UserId,
        workspace: WorkspaceId,
    ) -> Result<AppProjectRecord, AppError> {
        let row=sqlx::query("SELECT workspace_id,name,created_at::text,updated_at::text FROM latex_core.projects WHERE workspace_id=$1 AND owner_user_id=$2").bind(workspace.as_uuid()).bind(owner.as_uuid()).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        decode_project(row)
    }
    pub async fn assert_project_owner(
        &self,
        owner: UserId,
        workspace: WorkspaceId,
    ) -> Result<(), AppError> {
        self.project(owner, workspace).await.map(|_| ())
    }
    pub async fn job(&self, owner: UserId, job: JobId) -> Result<AppJobRecord, AppError> {
        let row=sqlx::query("SELECT j.id,j.workspace_id,j.state,j.snapshot_id,j.created_at::text,j.finished_at::text,j.last_error,CASE WHEN j.state='queued' THEN 1 + (SELECT count(*) FROM latex_core.compile_jobs q WHERE q.state='queued' AND (q.priority>j.priority OR (q.priority=j.priority AND (q.created_at<j.created_at OR (q.created_at=j.created_at AND q.id<j.id))))) ELSE NULL END AS queue_position,CASE WHEN j.state='queued' THEN (SELECT count(*) FROM latex_core.compile_jobs q WHERE q.state='queued' AND (q.priority>j.priority OR (q.priority=j.priority AND (q.created_at<j.created_at OR (q.created_at=j.created_at AND q.id<j.id))))) ELSE NULL END AS jobs_ahead FROM latex_core.compile_jobs j LEFT JOIN latex_core.projects p ON p.workspace_id=j.workspace_id AND p.owner_user_id=$2 LEFT JOIN latex_core.team_projects tp ON tp.workspace_id=j.workspace_id LEFT JOIN latex_core.team_project_members pm ON pm.team_project_id=tp.id AND pm.user_id=$2 LEFT JOIN latex_core.research_groups rg ON rg.workspace_id=j.workspace_id LEFT JOIN latex_core.research_group_members rgm ON rgm.group_id=rg.id AND rgm.user_id=$2 WHERE j.id=$1 AND (p.workspace_id IS NOT NULL OR pm.user_id IS NOT NULL OR rgm.user_id IS NOT NULL)").bind(job.as_uuid()).bind(owner.as_uuid()).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        decode_job(row)
    }
    pub async fn artifacts(
        &self,
        owner: UserId,
        job: JobId,
    ) -> Result<Vec<AppArtifactRecord>, AppError> {
        let job_record = self.job(owner, job).await?;
        if !self
            .effective_permissions(owner, job_record.workspace_id)
            .await?
            .allows(Permission::ArtifactRead)
        {
            return Err(AppError::Forbidden);
        }
        let rows=sqlx::query("SELECT artifact_id,logical_name,blob_hash,size_bytes,content_type FROM latex_core.compilation_artifacts WHERE job_id=$1 ORDER BY logical_name").bind(job.as_uuid()).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_artifact).collect()
    }
    pub async fn artifact(
        &self,
        owner: UserId,
        job: JobId,
        artifact: ArtifactId,
    ) -> Result<AppArtifactRecord, AppError> {
        let job_record = self.job(owner, job).await?;
        if !self
            .effective_permissions(owner, job_record.workspace_id)
            .await?
            .allows(Permission::ArtifactRead)
        {
            return Err(AppError::Forbidden);
        }
        let row=sqlx::query("SELECT artifact_id,logical_name,blob_hash,size_bytes,content_type FROM latex_core.compilation_artifacts WHERE job_id=$1 AND artifact_id=$2").bind(job.as_uuid()).bind(artifact.as_uuid()).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        decode_artifact(row)
    }
}
pub(crate) fn map_conflict(error: sqlx::Error) -> AppError {
    if matches!(error.as_database_error().and_then(sqlx::error::DatabaseError::code), Some(code) if code == "23505")
    {
        AppError::Conflict
    } else {
        AppError::Database(error)
    }
}
fn decode_user(r: sqlx::postgres::PgRow) -> Result<AppUserRecord, AppError> {
    Ok(AppUserRecord {
        user_id: UserId::from_uuid(r.try_get("id").map_err(AppError::Database)?),
        tenant_id: TenantId::from_uuid(r.try_get("tenant_id").map_err(AppError::Database)?),
        email: r.try_get("email").map_err(AppError::Database)?,
        password_hash: r.try_get("password_hash").map_err(AppError::Database)?,
        enabled: r.try_get("enabled").map_err(AppError::Database)?,
        account_type: r.try_get("account_type").map_err(AppError::Database)?,
    })
}
fn decode_project(r: sqlx::postgres::PgRow) -> Result<AppProjectRecord, AppError> {
    Ok(AppProjectRecord {
        workspace_id: WorkspaceId::from_uuid(
            r.try_get("workspace_id").map_err(AppError::Database)?,
        ),
        name: r.try_get("name").map_err(AppError::Database)?,
        created_at: r.try_get("created_at").map_err(AppError::Database)?,
        updated_at: r.try_get("updated_at").map_err(AppError::Database)?,
    })
}
fn decode_job(r: sqlx::postgres::PgRow) -> Result<AppJobRecord, AppError> {
    Ok(AppJobRecord {
        id: JobId::from_uuid(r.try_get("id").map_err(AppError::Database)?),
        workspace_id: WorkspaceId::from_uuid(
            r.try_get("workspace_id").map_err(AppError::Database)?,
        ),
        state: r.try_get("state").map_err(AppError::Database)?,
        snapshot_id: r.try_get("snapshot_id").map_err(AppError::Database)?,
        created_at: r.try_get("created_at").map_err(AppError::Database)?,
        finished_at: r.try_get("finished_at").map_err(AppError::Database)?,
        last_error: r.try_get("last_error").map_err(AppError::Database)?,
        queue_position: r
            .try_get::<Option<i64>, _>("queue_position")
            .map_err(AppError::Database)?
            .map(|value| {
                u64::try_from(value).map_err(|_| AppError::Integrity {
                    message: "negative queue position".into(),
                })
            })
            .transpose()?,
        jobs_ahead: r
            .try_get::<Option<i64>, _>("jobs_ahead")
            .map_err(AppError::Database)?
            .map(|value| {
                u64::try_from(value).map_err(|_| AppError::Integrity {
                    message: "negative jobs ahead".into(),
                })
            })
            .transpose()?,
    })
}
fn decode_artifact(r: sqlx::postgres::PgRow) -> Result<AppArtifactRecord, AppError> {
    let size: i64 = r.try_get("size_bytes").map_err(AppError::Database)?;
    Ok(AppArtifactRecord {
        id: ArtifactId::from_uuid(r.try_get("artifact_id").map_err(AppError::Database)?),
        logical_name: r.try_get("logical_name").map_err(AppError::Database)?,
        blob_hash: BlobHash::from_str(
            &r.try_get::<String, _>("blob_hash")
                .map_err(AppError::Database)?,
        )
        .map_err(|e| AppError::Integrity {
            message: e.to_string(),
        })?,
        size_bytes: u64::try_from(size).map_err(|_| AppError::Integrity {
            message: "negative artifact size".into(),
        })?,
        content_type: r.try_get("content_type").map_err(AppError::Database)?,
    })
}
fn decode_template(r: sqlx::postgres::PgRow) -> Result<AppTemplateRecord, AppError> {
    Ok(AppTemplateRecord {
        id: r.try_get("id").map_err(AppError::Database)?,
        name: r.try_get("name").map_err(AppError::Database)?,
        description: r.try_get("description").map_err(AppError::Database)?,
        main_file: r.try_get("main_file").map_err(AppError::Database)?,
        created_at: r.try_get("created_at").map_err(AppError::Database)?,
    })
}
fn decode_template_file(r: sqlx::postgres::PgRow) -> Result<AppTemplateFileRecord, AppError> {
    let size: i64 = r.try_get("size_bytes").map_err(AppError::Database)?;
    Ok(AppTemplateFileRecord {
        path: r.try_get("path").map_err(AppError::Database)?,
        blob_hash: BlobHash::from_str(
            &r.try_get::<String, _>("blob_hash")
                .map_err(AppError::Database)?,
        )
        .map_err(|error| AppError::Integrity {
            message: error.to_string(),
        })?,
        size_bytes: u64::try_from(size).map_err(|_| AppError::Integrity {
            message: "negative template file size".into(),
        })?,
    })
}
fn decode_research_group(r: sqlx::postgres::PgRow) -> Result<ResearchGroupRecord, AppError> {
    Ok(ResearchGroupRecord {
        id: r.try_get("id").map_err(AppError::Database)?,
        name: r.try_get("name").map_err(AppError::Database)?,
        owner_user_id: UserId::from_uuid(r.try_get("owner_user_id").map_err(AppError::Database)?),
        workspace_id: WorkspaceId::from_uuid(
            r.try_get("workspace_id").map_err(AppError::Database)?,
        ),
        created_at: r.try_get("created_at").map_err(AppError::Database)?,
        updated_at: r.try_get("updated_at").map_err(AppError::Database)?,
    })
}
