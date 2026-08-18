//! Durable authorization and canonical team-project collaboration records.
//!
//! Team drafts are intentionally separate from workspace events. Only a successful
//! publish appends a workspace mutation, so the existing snapshot/compiler pipeline
//! can only observe the canonical team head.
#![allow(
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    reason = "repository methods share AppError and SQL row decoding consumes values at call sites"
)]

use crate::{AppError, AppRepository};
use core_types::{BlobHash, UserId, WorkspaceId};
use serde_json::json;
use sqlx::{Postgres, Row, Transaction};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AccountType {
    Student,
    Professor,
    Admin,
}
impl AccountType {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "student" => Ok(Self::Student),
            "professor" => Ok(Self::Professor),
            "admin" => Ok(Self::Admin),
            _ => Err(AppError::Integrity {
                message: "invalid institutional account type".into(),
            }),
        }
    }
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Student => "student",
            Self::Professor => "professor",
            Self::Admin => "admin",
        }
    }
    #[must_use]
    pub const fn is_admin(self) -> bool {
        matches!(self, Self::Admin)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FilePolicy {
    Editable,
    ReadOnly,
    Hidden,
    AdminOnly,
}
impl FilePolicy {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "editable" => Ok(Self::Editable),
            "read_only" => Ok(Self::ReadOnly),
            "hidden" => Ok(Self::Hidden),
            "admin_only" => Ok(Self::AdminOnly),
            _ => Err(AppError::Integrity {
                message: "invalid file policy".into(),
            }),
        }
    }
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Editable => "editable",
            Self::ReadOnly => "read_only",
            Self::Hidden => "hidden",
            Self::AdminOnly => "admin_only",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TeamRecord {
    pub id: Uuid,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug)]
pub struct TeamMemberRecord {
    pub user_id: UserId,
    pub email: String,
    pub can_write: bool,
    pub can_mentor: bool,
    pub can_manage: bool,
}
#[derive(Clone, Debug)]
pub struct TeamProjectRecord {
    pub id: Uuid,
    pub team_id: Uuid,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub canonical_generation: u64,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug)]
pub struct TeamFileRecord {
    pub path: String,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
    pub revision: u64,
    pub policy: FilePolicy,
}
#[derive(Clone, Debug)]
pub struct MemberDraftRecord {
    pub path: String,
    pub base_revision: u64,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
}
#[derive(Clone, Debug)]
pub enum ProjectAccess {
    Personal {
        account_type: AccountType,
    },
    Team {
        project: TeamProjectRecord,
        account_type: AccountType,
        can_write: bool,
        can_mentor: bool,
        can_manage: bool,
    },
}
impl ProjectAccess {
    #[must_use]
    pub const fn account_type(&self) -> AccountType {
        match self {
            Self::Personal { account_type } | Self::Team { account_type, .. } => *account_type,
        }
    }
    #[must_use]
    pub const fn is_team(&self) -> bool {
        matches!(self, Self::Team { .. })
    }
}
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PublishResult {
    Published {
        canonical_generation: u64,
        workspace_version: u64,
        file_revision: u64,
    },
    Conflict {
        current_file_revision: u64,
    },
}

impl AppRepository {
    pub async fn account_type(&self, user: UserId) -> Result<AccountType, AppError> {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT account_type FROM latex_core.user_credentials WHERE user_id=$1",
        )
        .bind(user.as_uuid())
        .fetch_optional(self.database.pool())
        .await
        .map_err(AppError::Database)?;
        AccountType::parse(&value.ok_or(AppError::NotFound)?)
    }

    pub async fn create_team(&self, creator: UserId, name: &str) -> Result<TeamRecord, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO latex_core.teams (id,name,created_by) VALUES ($1,$2,$3)")
            .bind(id)
            .bind(name)
            .bind(creator.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(crate::app::map_conflict)?;
        sqlx::query("INSERT INTO latex_core.team_members (team_id,user_id,can_write,can_mentor,can_manage) VALUES ($1,$2,TRUE,FALSE,TRUE)").bind(id).bind(creator.as_uuid()).execute(&mut *tx).await.map_err(AppError::Database)?;
        let row = sqlx::query(
            "SELECT id,name,created_at::text,updated_at::text FROM latex_core.teams WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        decode_team(row)
    }

    pub async fn teams_for_user(&self, user: UserId) -> Result<Vec<TeamRecord>, AppError> {
        let rows = sqlx::query("SELECT t.id,t.name,t.created_at::text,t.updated_at::text FROM latex_core.teams t JOIN latex_core.team_members m ON m.team_id=t.id WHERE m.user_id=$1 ORDER BY t.updated_at DESC,t.id")
            .bind(user.as_uuid()).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_team).collect()
    }

    pub async fn team(&self, actor: UserId, team_id: Uuid) -> Result<TeamRecord, AppError> {
        self.assert_team_visible(actor, team_id).await?;
        let row = sqlx::query(
            "SELECT id,name,created_at::text,updated_at::text FROM latex_core.teams WHERE id=$1",
        )
        .bind(team_id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(AppError::Database)?
        .ok_or(AppError::NotFound)?;
        decode_team(row)
    }

    pub async fn team_members(
        &self,
        actor: UserId,
        team_id: Uuid,
    ) -> Result<Vec<TeamMemberRecord>, AppError> {
        self.assert_team_visible(actor, team_id).await?;
        let rows = sqlx::query("SELECT m.user_id,c.email,m.can_write,m.can_mentor,m.can_manage FROM latex_core.team_members m JOIN latex_core.user_credentials c ON c.user_id=m.user_id WHERE m.team_id=$1 ORDER BY c.email")
            .bind(team_id).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_member).collect()
    }

    pub async fn set_team_member(
        &self,
        actor: UserId,
        team_id: Uuid,
        target: UserId,
        can_write: bool,
        can_mentor: bool,
        can_manage: bool,
    ) -> Result<(), AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        assert_manager(&mut tx, actor, team_id).await?;
        sqlx::query("INSERT INTO latex_core.team_members (team_id,user_id,can_write,can_mentor,can_manage) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (team_id,user_id) DO UPDATE SET can_write=EXCLUDED.can_write,can_mentor=EXCLUDED.can_mentor,can_manage=EXCLUDED.can_manage")
            .bind(team_id).bind(target.as_uuid()).bind(can_write).bind(can_mentor).bind(can_manage).execute(&mut *tx).await.map_err(AppError::Database)?;
        sqlx::query("UPDATE latex_core.teams SET updated_at=statement_timestamp() WHERE id=$1")
            .bind(team_id)
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)
    }

    pub async fn remove_team_member(
        &self,
        actor: UserId,
        team_id: Uuid,
        target: UserId,
    ) -> Result<(), AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        assert_manager(&mut tx, actor, team_id).await?;
        let result =
            sqlx::query("DELETE FROM latex_core.team_members WHERE team_id=$1 AND user_id=$2")
                .bind(team_id)
                .bind(target.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(AppError::Database)?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        tx.commit().await.map_err(AppError::Database)
    }

    pub async fn create_team_project(
        &self,
        actor: UserId,
        team_id: Uuid,
        workspace: WorkspaceId,
        name: &str,
        files: &[TeamFileRecord],
    ) -> Result<TeamProjectRecord, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        assert_manager(&mut tx, actor, team_id).await?;
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO latex_core.team_projects (id,team_id,workspace_id,name) VALUES ($1,$2,$3,$4)").bind(id).bind(team_id).bind(workspace.as_uuid()).bind(name).execute(&mut *tx).await.map_err(crate::app::map_conflict)?;
        for file in files {
            sqlx::query("INSERT INTO latex_core.team_project_files (team_project_id,logical_path,blob_hash,size_bytes,file_revision) VALUES ($1,$2,$3,$4,$5)")
                .bind(id).bind(&file.path).bind(file.blob_hash.to_hex()).bind(i64_size(file.size_bytes)?).bind(i64_revision(file.revision)?).execute(&mut *tx).await.map_err(AppError::Database)?;
        }
        let row = sqlx::query("SELECT id,team_id,workspace_id,name,canonical_generation,created_at::text,updated_at::text FROM latex_core.team_projects WHERE id=$1").bind(id).fetch_one(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        decode_team_project(row)
    }

    pub async fn team_projects(
        &self,
        actor: UserId,
        team_id: Uuid,
    ) -> Result<Vec<TeamProjectRecord>, AppError> {
        self.assert_team_visible(actor, team_id).await?;
        let rows = sqlx::query("SELECT id,team_id,workspace_id,name,canonical_generation,created_at::text,updated_at::text FROM latex_core.team_projects WHERE team_id=$1 ORDER BY updated_at DESC,id").bind(team_id).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_team_project).collect()
    }

    pub async fn project_access(
        &self,
        user: UserId,
        workspace: WorkspaceId,
    ) -> Result<ProjectAccess, AppError> {
        let account_type = self.account_type(user).await?;
        let personal: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.projects WHERE workspace_id=$1 AND owner_user_id=$2)").bind(workspace.as_uuid()).bind(user.as_uuid()).fetch_one(self.database.pool()).await.map_err(AppError::Database)?;
        if personal {
            return Ok(ProjectAccess::Personal { account_type });
        }
        let row = sqlx::query("SELECT tp.id,tp.team_id,tp.workspace_id,tp.name,tp.canonical_generation,tp.created_at::text,tp.updated_at::text,COALESCE(m.can_write,FALSE) AS can_write,COALESCE(m.can_mentor,FALSE) AS can_mentor,COALESCE(m.can_manage,FALSE) AS can_manage FROM latex_core.team_projects tp LEFT JOIN latex_core.team_members m ON m.team_id=tp.team_id AND m.user_id=$2 WHERE tp.workspace_id=$1 AND ($3 OR m.user_id IS NOT NULL)")
            .bind(workspace.as_uuid()).bind(user.as_uuid()).bind(account_type.is_admin()).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        let can_write = row.try_get("can_write").map_err(AppError::Database)?;
        let can_mentor = row.try_get("can_mentor").map_err(AppError::Database)?;
        let can_manage = row.try_get("can_manage").map_err(AppError::Database)?;
        Ok(ProjectAccess::Team {
            project: decode_team_project(row)?,
            account_type,
            can_write,
            can_mentor,
            can_manage,
        })
    }

    pub async fn team_files_for_user(
        &self,
        user: UserId,
        project_id: Uuid,
    ) -> Result<Vec<TeamFileRecord>, AppError> {
        let access = self.team_access_by_id(user, project_id).await?;
        let (account, manage) = match access {
            ProjectAccess::Team {
                account_type,
                can_manage,
                ..
            } => (account_type, can_manage),
            ProjectAccess::Personal { .. } => return Err(AppError::NotFound),
        };
        let rows = sqlx::query("SELECT f.logical_path,f.blob_hash,f.size_bytes,f.file_revision,COALESCE(p.access_policy,'editable') AS access_policy FROM latex_core.team_project_files f LEFT JOIN latex_core.file_policies p ON p.team_project_id=f.team_project_id AND p.logical_path=f.logical_path WHERE f.team_project_id=$1 ORDER BY f.logical_path")
            .bind(project_id).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter()
            .filter_map(|row| match decode_team_file(row) {
                Ok(file) if may_read(file.policy, account, manage) => Some(Ok(file)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    pub async fn team_file_for_user(
        &self,
        user: UserId,
        project_id: Uuid,
        path: &str,
    ) -> Result<TeamFileRecord, AppError> {
        let access = self.team_access_by_id(user, project_id).await?;
        let (account, manage) = match access {
            ProjectAccess::Team {
                account_type,
                can_manage,
                ..
            } => (account_type, can_manage),
            ProjectAccess::Personal { .. } => return Err(AppError::NotFound),
        };
        let row = sqlx::query("SELECT f.logical_path,f.blob_hash,f.size_bytes,f.file_revision,COALESCE(p.access_policy,'editable') AS access_policy FROM latex_core.team_project_files f LEFT JOIN latex_core.file_policies p ON p.team_project_id=f.team_project_id AND p.logical_path=f.logical_path WHERE f.team_project_id=$1 AND f.logical_path=$2")
            .bind(project_id).bind(path).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        let file = decode_team_file(row)?;
        if may_read(file.policy, account, manage) {
            Ok(file)
        } else {
            Err(AppError::NotFound)
        }
    }

    pub async fn draft_for_user(
        &self,
        user: UserId,
        project_id: Uuid,
        path: &str,
    ) -> Result<Option<MemberDraftRecord>, AppError> {
        let row = sqlx::query("SELECT logical_path,base_file_revision,draft_blob_hash,draft_size_bytes FROM latex_core.member_drafts WHERE team_project_id=$1 AND user_id=$2 AND logical_path=$3")
            .bind(project_id).bind(user.as_uuid()).bind(path).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?;
        row.map(decode_draft).transpose()
    }

    pub async fn save_draft(
        &self,
        user: UserId,
        project_id: Uuid,
        path: &str,
        base_revision: u64,
        blob_hash: BlobHash,
        size_bytes: u64,
    ) -> Result<(), AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let access = team_access_tx(&mut tx, user, project_id).await?;
        assert_mutable(access, path, &mut tx, project_id).await?;
        sqlx::query("INSERT INTO latex_core.member_drafts (team_project_id,user_id,logical_path,base_file_revision,draft_blob_hash,draft_size_bytes) VALUES ($1,$2,$3,$4,$5,$6) ON CONFLICT (team_project_id,user_id,logical_path) DO UPDATE SET base_file_revision=EXCLUDED.base_file_revision,draft_blob_hash=EXCLUDED.draft_blob_hash,draft_size_bytes=EXCLUDED.draft_size_bytes,updated_at=statement_timestamp()")
            .bind(project_id).bind(user.as_uuid()).bind(path).bind(i64_revision(base_revision)?).bind(blob_hash.to_hex()).bind(i64_size(size_bytes)?).execute(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)
    }

    pub async fn publish_draft(
        &self,
        user: UserId,
        project_id: Uuid,
        path: &str,
    ) -> Result<PublishResult, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let access = team_access_tx(&mut tx, user, project_id).await?;
        assert_mutable(access, path, &mut tx, project_id).await?;
        let project = sqlx::query("SELECT id,team_id,workspace_id,name,canonical_generation,created_at::text,updated_at::text FROM latex_core.team_projects WHERE id=$1 FOR UPDATE").bind(project_id).fetch_optional(&mut *tx).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        let project = decode_team_project(project)?;
        let draft = sqlx::query("SELECT logical_path,base_file_revision,draft_blob_hash,draft_size_bytes FROM latex_core.member_drafts WHERE team_project_id=$1 AND user_id=$2 AND logical_path=$3 FOR UPDATE").bind(project_id).bind(user.as_uuid()).bind(path).fetch_optional(&mut *tx).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        let draft = decode_draft(draft)?;
        let canonical = sqlx::query("SELECT blob_hash,file_revision FROM latex_core.team_project_files WHERE team_project_id=$1 AND logical_path=$2 FOR UPDATE").bind(project_id).bind(path).fetch_optional(&mut *tx).await.map_err(AppError::Database)?;
        let (previous, current_revision) = match canonical {
            Some(row) => (
                Some(
                    row.try_get::<String, _>("blob_hash")
                        .map_err(AppError::Database)?,
                ),
                db_revision(row.try_get("file_revision").map_err(AppError::Database)?)?,
            ),
            None => (None, 0),
        };
        if draft.base_revision != current_revision {
            tx.commit().await.map_err(AppError::Database)?;
            return Ok(PublishResult::Conflict {
                current_file_revision: current_revision,
            });
        }
        let next_revision = current_revision
            .checked_add(1)
            .ok_or_else(|| AppError::Integrity {
                message: "file revision overflow".into(),
            })?;
        sqlx::query("INSERT INTO latex_core.team_project_files (team_project_id,logical_path,blob_hash,size_bytes,file_revision) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (team_project_id,logical_path) DO UPDATE SET blob_hash=EXCLUDED.blob_hash,size_bytes=EXCLUDED.size_bytes,file_revision=EXCLUDED.file_revision")
            .bind(project_id).bind(path).bind(draft.blob_hash.to_hex()).bind(i64_size(draft.size_bytes)?).bind(i64_revision(next_revision)?).execute(&mut *tx).await.map_err(AppError::Database)?;
        let workspace_version = append_workspace_put(
            &mut tx,
            project.workspace_id,
            user,
            path,
            draft.blob_hash,
            draft.size_bytes,
        )
        .await?;
        let generation = increment_generation(&mut tx, project_id).await?;
        audit(
            &mut tx,
            project_id,
            user,
            "publish",
            Some(path),
            previous.as_deref(),
            Some(&draft.blob_hash.to_hex()),
            generation,
        )
        .await?;
        sqlx::query("DELETE FROM latex_core.member_drafts WHERE team_project_id=$1 AND user_id=$2 AND logical_path=$3").bind(project_id).bind(user.as_uuid()).bind(path).execute(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(PublishResult::Published {
            canonical_generation: generation,
            workspace_version,
            file_revision: next_revision,
        })
    }

    pub async fn set_file_policy(
        &self,
        actor: UserId,
        project_id: Uuid,
        path: &str,
        policy: FilePolicy,
    ) -> Result<(), AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let access = team_access_tx(&mut tx, actor, project_id).await?;
        if !access.account_type().is_admin()
            && !matches!(
                access,
                ProjectAccess::Team {
                    can_manage: true,
                    ..
                }
            )
        {
            return Err(AppError::Forbidden);
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.team_project_files WHERE team_project_id=$1 AND logical_path=$2)").bind(project_id).bind(path).fetch_one(&mut *tx).await.map_err(AppError::Database)?;
        if !exists {
            return Err(AppError::NotFound);
        }
        sqlx::query("INSERT INTO latex_core.file_policies (team_project_id,logical_path,access_policy,set_by_user_id) VALUES ($1,$2,$3,$4) ON CONFLICT (team_project_id,logical_path) DO UPDATE SET access_policy=EXCLUDED.access_policy,set_by_user_id=EXCLUDED.set_by_user_id,updated_at=statement_timestamp()")
            .bind(project_id).bind(path).bind(policy.as_str()).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)
    }

    pub async fn delete_team_file(
        &self,
        actor: UserId,
        project_id: Uuid,
        path: &str,
    ) -> Result<u64, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let access = team_access_tx(&mut tx, actor, project_id).await?;
        assert_mutable(access, path, &mut tx, project_id).await?;
        let project = locked_project(&mut tx, project_id).await?;
        let previous: String = sqlx::query_scalar("SELECT blob_hash FROM latex_core.team_project_files WHERE team_project_id=$1 AND logical_path=$2 FOR UPDATE").bind(project_id).bind(path).fetch_optional(&mut *tx).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        let workspace_version = append_workspace_operation(
            &mut tx,
            project.workspace_id,
            actor,
            json!({"op":"delete_file","path":path}),
        )
        .await?;
        sqlx::query("DELETE FROM latex_core.team_project_files WHERE team_project_id=$1 AND logical_path=$2").bind(project_id).bind(path).execute(&mut *tx).await.map_err(AppError::Database)?;
        sqlx::query(
            "DELETE FROM latex_core.file_policies WHERE team_project_id=$1 AND logical_path=$2",
        )
        .bind(project_id)
        .bind(path)
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        let generation = increment_generation(&mut tx, project_id).await?;
        audit(
            &mut tx,
            project_id,
            actor,
            "delete",
            Some(path),
            Some(&previous),
            None,
            generation,
        )
        .await?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(workspace_version)
    }

    pub async fn rename_team_file(
        &self,
        actor: UserId,
        project_id: Uuid,
        from: &str,
        to: &str,
    ) -> Result<u64, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let access = team_access_tx(&mut tx, actor, project_id).await?;
        assert_mutable(access, from, &mut tx, project_id).await?;
        let project = locked_project(&mut tx, project_id).await?;
        let previous: String = sqlx::query_scalar("SELECT blob_hash FROM latex_core.team_project_files WHERE team_project_id=$1 AND logical_path=$2 FOR UPDATE").bind(project_id).bind(from).fetch_optional(&mut *tx).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        let collision: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.team_project_files WHERE team_project_id=$1 AND logical_path=$2)").bind(project_id).bind(to).fetch_one(&mut *tx).await.map_err(AppError::Database)?;
        if collision && from != to {
            return Err(AppError::Conflict);
        }
        let workspace_version = append_workspace_operation(
            &mut tx,
            project.workspace_id,
            actor,
            json!({"op":"rename_file","from":from,"to":to}),
        )
        .await?;
        if from != to {
            sqlx::query("UPDATE latex_core.team_project_files SET logical_path=$3 WHERE team_project_id=$1 AND logical_path=$2").bind(project_id).bind(from).bind(to).execute(&mut *tx).await.map_err(AppError::Database)?;
            sqlx::query("UPDATE latex_core.file_policies SET logical_path=$3 WHERE team_project_id=$1 AND logical_path=$2").bind(project_id).bind(from).bind(to).execute(&mut *tx).await.map_err(AppError::Database)?;
        }
        let generation = increment_generation(&mut tx, project_id).await?;
        audit(
            &mut tx,
            project_id,
            actor,
            "rename",
            Some(to),
            Some(&previous),
            Some(&previous),
            generation,
        )
        .await?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(workspace_version)
    }

    pub async fn set_team_main_file(
        &self,
        actor: UserId,
        project_id: Uuid,
        path: &str,
    ) -> Result<u64, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let access = team_access_tx(&mut tx, actor, project_id).await?;
        assert_mutable(access, path, &mut tx, project_id).await?;
        let project = locked_project(&mut tx, project_id).await?;
        let hash: String = sqlx::query_scalar("SELECT blob_hash FROM latex_core.team_project_files WHERE team_project_id=$1 AND logical_path=$2 FOR UPDATE").bind(project_id).bind(path).fetch_optional(&mut *tx).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        let workspace_version = append_workspace_operation(
            &mut tx,
            project.workspace_id,
            actor,
            json!({"op":"set_main_file","path":path}),
        )
        .await?;
        let generation = increment_generation(&mut tx, project_id).await?;
        audit(
            &mut tx,
            project_id,
            actor,
            "set_main",
            Some(path),
            Some(&hash),
            Some(&hash),
            generation,
        )
        .await?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(workspace_version)
    }

    async fn team_access_by_id(
        &self,
        user: UserId,
        project_id: Uuid,
    ) -> Result<ProjectAccess, AppError> {
        let workspace: Uuid =
            sqlx::query_scalar("SELECT workspace_id FROM latex_core.team_projects WHERE id=$1")
                .bind(project_id)
                .fetch_optional(self.database.pool())
                .await
                .map_err(AppError::Database)?
                .ok_or(AppError::NotFound)?;
        self.project_access(user, WorkspaceId::from_uuid(workspace))
            .await
    }
    async fn assert_team_visible(&self, user: UserId, team_id: Uuid) -> Result<(), AppError> {
        let account = self.account_type(user).await?;
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM latex_core.team_members WHERE team_id=$1 AND user_id=$2)",
        )
        .bind(team_id)
        .bind(user.as_uuid())
        .fetch_one(self.database.pool())
        .await
        .map_err(AppError::Database)?;
        if member || account.is_admin() {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }
}

async fn team_access_tx(
    tx: &mut Transaction<'_, Postgres>,
    user: UserId,
    project_id: Uuid,
) -> Result<ProjectAccess, AppError> {
    let account: String =
        sqlx::query_scalar("SELECT account_type FROM latex_core.user_credentials WHERE user_id=$1")
            .bind(user.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?
            .ok_or(AppError::NotFound)?;
    let account = AccountType::parse(&account)?;
    let row = sqlx::query("SELECT tp.id,tp.team_id,tp.workspace_id,tp.name,tp.canonical_generation,tp.created_at::text,tp.updated_at::text,COALESCE(m.can_write,FALSE) AS can_write,COALESCE(m.can_mentor,FALSE) AS can_mentor,COALESCE(m.can_manage,FALSE) AS can_manage FROM latex_core.team_projects tp LEFT JOIN latex_core.team_members m ON m.team_id=tp.team_id AND m.user_id=$2 WHERE tp.id=$1 AND ($3 OR m.user_id IS NOT NULL)").bind(project_id).bind(user.as_uuid()).bind(account.is_admin()).fetch_optional(&mut **tx).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
    let can_write = row.try_get("can_write").map_err(AppError::Database)?;
    let can_mentor = row.try_get("can_mentor").map_err(AppError::Database)?;
    let can_manage = row.try_get("can_manage").map_err(AppError::Database)?;
    Ok(ProjectAccess::Team {
        project: decode_team_project(row)?,
        account_type: account,
        can_write,
        can_mentor,
        can_manage,
    })
}
async fn assert_manager(
    tx: &mut Transaction<'_, Postgres>,
    user: UserId,
    team_id: Uuid,
) -> Result<(), AppError> {
    let admin: bool = sqlx::query_scalar(
        "SELECT account_type='admin' FROM latex_core.user_credentials WHERE user_id=$1",
    )
    .bind(user.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(AppError::Database)?
    .ok_or(AppError::NotFound)?;
    let manager: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.team_members WHERE team_id=$1 AND user_id=$2 AND can_manage)").bind(team_id).bind(user.as_uuid()).fetch_one(&mut **tx).await.map_err(AppError::Database)?;
    if admin || manager {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}
async fn assert_mutable(
    access: ProjectAccess,
    path: &str,
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
) -> Result<(), AppError> {
    let (account, write) = match access {
        ProjectAccess::Team {
            account_type,
            can_write,
            ..
        } => (account_type, can_write),
        ProjectAccess::Personal { .. } => return Err(AppError::NotFound),
    };
    let policy: Option<String> = sqlx::query_scalar("SELECT access_policy FROM latex_core.file_policies WHERE team_project_id=$1 AND logical_path=$2").bind(project_id).bind(path).fetch_optional(&mut **tx).await.map_err(AppError::Database)?;
    if account.is_admin() || (write && policy.as_deref().is_none_or(|value| value == "editable")) {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}
fn may_read(policy: FilePolicy, account: AccountType, manager: bool) -> bool {
    account.is_admin()
        || match policy {
            FilePolicy::Editable | FilePolicy::ReadOnly => true,
            FilePolicy::Hidden => manager,
            FilePolicy::AdminOnly => false,
        }
}
async fn append_workspace_put(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    actor: UserId,
    path: &str,
    hash: BlobHash,
    size: u64,
) -> Result<u64, AppError> {
    append_workspace_operation(
        tx,
        workspace,
        actor,
        json!({"op":"put_file","path":path,"blob_hash":hash.to_hex(),"size_bytes":size}),
    )
    .await
}
async fn append_workspace_operation(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    actor: UserId,
    operation: serde_json::Value,
) -> Result<u64, AppError> {
    let actual: i64 = sqlx::query_scalar(
        "SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1 FOR UPDATE",
    )
    .bind(workspace.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(AppError::Database)?
    .ok_or(AppError::NotFound)?;
    let next = actual.checked_add(1).ok_or_else(|| AppError::Integrity {
        message: "workspace version overflow".into(),
    })?;
    let payload = json!({"schema_version":1,"operations":[operation]});
    sqlx::query("INSERT INTO latex_core.workspace_events (workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) VALUES ($1,$2,$3,$4,'workspace.mutation',1,$5,$6)").bind(workspace.as_uuid()).bind(next).bind(Uuid::new_v4()).bind(actual).bind(payload).bind(actor.as_uuid()).execute(&mut **tx).await.map_err(AppError::Database)?;
    sqlx::query("UPDATE latex_core.workspace_heads SET durable_version=$2,updated_at=statement_timestamp() WHERE workspace_id=$1").bind(workspace.as_uuid()).bind(next).execute(&mut **tx).await.map_err(AppError::Database)?;
    u64::try_from(next).map_err(|_| AppError::Integrity {
        message: "negative workspace version".into(),
    })
}
async fn locked_project(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
) -> Result<TeamProjectRecord, AppError> {
    let row = sqlx::query("SELECT id,team_id,workspace_id,name,canonical_generation,created_at::text,updated_at::text FROM latex_core.team_projects WHERE id=$1 FOR UPDATE").bind(project_id).fetch_optional(&mut **tx).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
    decode_team_project(row)
}
async fn increment_generation(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
) -> Result<u64, AppError> {
    let generation: i64 = sqlx::query_scalar("UPDATE latex_core.team_projects SET canonical_generation=canonical_generation+1,updated_at=statement_timestamp() WHERE id=$1 RETURNING canonical_generation").bind(project_id).fetch_one(&mut **tx).await.map_err(AppError::Database)?;
    db_revision(generation)
}
async fn audit(
    tx: &mut Transaction<'_, Postgres>,
    project: Uuid,
    actor: UserId,
    action: &str,
    path: Option<&str>,
    previous: Option<&str>,
    new: Option<&str>,
    generation: u64,
) -> Result<(), AppError> {
    sqlx::query("INSERT INTO latex_core.team_project_audit (id,team_project_id,actor_user_id,action_type,logical_path,previous_blob_hash,new_blob_hash,canonical_generation) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)").bind(Uuid::new_v4()).bind(project).bind(actor.as_uuid()).bind(action).bind(path).bind(previous).bind(new).bind(i64_revision(generation)?).execute(&mut **tx).await.map_err(AppError::Database)?;
    Ok(())
}
fn decode_team(row: sqlx::postgres::PgRow) -> Result<TeamRecord, AppError> {
    Ok(TeamRecord {
        id: row.try_get("id").map_err(AppError::Database)?,
        name: row.try_get("name").map_err(AppError::Database)?,
        created_at: row.try_get("created_at").map_err(AppError::Database)?,
        updated_at: row.try_get("updated_at").map_err(AppError::Database)?,
    })
}
fn decode_member(row: sqlx::postgres::PgRow) -> Result<TeamMemberRecord, AppError> {
    Ok(TeamMemberRecord {
        user_id: UserId::from_uuid(row.try_get("user_id").map_err(AppError::Database)?),
        email: row.try_get("email").map_err(AppError::Database)?,
        can_write: row.try_get("can_write").map_err(AppError::Database)?,
        can_mentor: row.try_get("can_mentor").map_err(AppError::Database)?,
        can_manage: row.try_get("can_manage").map_err(AppError::Database)?,
    })
}
fn decode_team_project(row: sqlx::postgres::PgRow) -> Result<TeamProjectRecord, AppError> {
    Ok(TeamProjectRecord {
        id: row.try_get("id").map_err(AppError::Database)?,
        team_id: row.try_get("team_id").map_err(AppError::Database)?,
        workspace_id: WorkspaceId::from_uuid(
            row.try_get("workspace_id").map_err(AppError::Database)?,
        ),
        name: row.try_get("name").map_err(AppError::Database)?,
        canonical_generation: db_revision(
            row.try_get("canonical_generation")
                .map_err(AppError::Database)?,
        )?,
        created_at: row.try_get("created_at").map_err(AppError::Database)?,
        updated_at: row.try_get("updated_at").map_err(AppError::Database)?,
    })
}
fn decode_team_file(row: sqlx::postgres::PgRow) -> Result<TeamFileRecord, AppError> {
    let size: i64 = row.try_get("size_bytes").map_err(AppError::Database)?;
    Ok(TeamFileRecord {
        path: row.try_get("logical_path").map_err(AppError::Database)?,
        blob_hash: BlobHash::from_str(
            &row.try_get::<String, _>("blob_hash")
                .map_err(AppError::Database)?,
        )
        .map_err(|error| AppError::Integrity {
            message: error.to_string(),
        })?,
        size_bytes: u64::try_from(size).map_err(|_| AppError::Integrity {
            message: "negative file size".into(),
        })?,
        revision: db_revision(row.try_get("file_revision").map_err(AppError::Database)?)?,
        policy: FilePolicy::parse(
            &row.try_get::<String, _>("access_policy")
                .map_err(AppError::Database)?,
        )?,
    })
}
fn decode_draft(row: sqlx::postgres::PgRow) -> Result<MemberDraftRecord, AppError> {
    let size: i64 = row
        .try_get("draft_size_bytes")
        .map_err(AppError::Database)?;
    Ok(MemberDraftRecord {
        path: row.try_get("logical_path").map_err(AppError::Database)?,
        base_revision: db_revision(
            row.try_get("base_file_revision")
                .map_err(AppError::Database)?,
        )?,
        blob_hash: BlobHash::from_str(
            &row.try_get::<String, _>("draft_blob_hash")
                .map_err(AppError::Database)?,
        )
        .map_err(|error| AppError::Integrity {
            message: error.to_string(),
        })?,
        size_bytes: u64::try_from(size).map_err(|_| AppError::Integrity {
            message: "negative draft size".into(),
        })?,
    })
}
fn i64_size(value: u64) -> Result<i64, AppError> {
    i64::try_from(value).map_err(|_| AppError::Integrity {
        message: "size exceeds PostgreSQL BIGINT".into(),
    })
}
fn i64_revision(value: u64) -> Result<i64, AppError> {
    i64::try_from(value).map_err(|_| AppError::Integrity {
        message: "revision exceeds PostgreSQL BIGINT".into(),
    })
}
fn db_revision(value: i64) -> Result<u64, AppError> {
    u64::try_from(value).map_err(|_| AppError::Integrity {
        message: "negative persisted revision".into(),
    })
}
