//! Additive V2 domain persistence. No V1 call path uses this repository in C3.

use crate::{
    Database,
    governance::{
        assert_content_policy, assert_main_policy, assert_structure_policy, workspace_mutation_lock,
    },
};
use core_types::{BlobHash, LogicalPath, TenantId, UserId, WorkspaceId};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};
use std::{collections::HashSet, str::FromStr};
use thiserror::Error;
use uuid::Uuid;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobalRole {
    Writer,
    Mentor,
    Admin,
}

impl GlobalRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Writer => "writer",
            Self::Mentor => "mentor",
            Self::Admin => "admin",
        }
    }
}

impl FromStr for GlobalRole {
    type Err = V2Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "writer" => Ok(Self::Writer),
            "mentor" => Ok(Self::Mentor),
            "admin" => Ok(Self::Admin),
            _ => Err(V2Error::InvalidRole {
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperStatus {
    Active,
    Frozen,
    Submitted,
    Archived,
}

impl PaperStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Frozen => "frozen",
            Self::Submitted => "submitted",
            Self::Archived => "archived",
        }
    }
}

impl FromStr for PaperStatus {
    type Err = V2Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "active" => Ok(Self::Active),
            "frozen" => Ok(Self::Frozen),
            "submitted" => Ok(Self::Submitted),
            "archived" => Ok(Self::Archived),
            _ => Err(V2Error::InvalidStatus {
                value: value.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GlobalRoleAssignment {
    pub user_id: UserId,
    pub role: GlobalRole,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersonalPaper {
    pub id: Uuid,
    pub owner_user_id: UserId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub status: PaperStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaperTeam {
    pub id: Uuid,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub status: PaperStatus,
    pub created_by_user_id: UserId,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaperTeamMember {
    pub paper_team_id: Uuid,
    pub user_id: UserId,
    pub assigned_by_user_id: UserId,
    pub is_leader: bool,
    pub writer_order: Option<i32>,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaperFile {
    pub file_id: Uuid,
    pub workspace_id: WorkspaceId,
    pub path: LogicalPath,
    pub revision: u64,
    pub tombstoned: bool,
    pub created_at: String,
    pub updated_at: String,
    pub tombstoned_at: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StructuralOperationResult {
    pub operation_id: Uuid,
    pub operation_type: String,
    pub file_id: Option<Uuid>,
    pub version: u64,
    pub state: String,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollaborationAccessMode {
    ReadWrite,
    ReadOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollaborationAccess {
    pub workspace_id: WorkspaceId,
    pub file: PaperFile,
    pub document_epoch: u64,
    pub mode: CollaborationAccessMode,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollaborationRecovery {
    pub document_epoch: u64,
    pub snapshot: Option<(u64, Vec<u8>)>,
    pub updates: Vec<(u64, Vec<u8>)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollaborationUpdateInput {
    pub actor_user_id: UserId,
    pub update_bytes: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct V2User {
    pub user_id: UserId,
    pub email: String,
    pub enabled: bool,
    pub legacy_account_type: String,
    pub v2_role: Option<GlobalRole>,
    pub migration_state: String,
    pub created_at: String,
    pub must_change_password: bool,
    pub email_delivery_id: Option<Uuid>,
    pub email_delivery_status: Option<String>,
    pub email_delivery_expired: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaperKind {
    Personal,
    Team,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WriterPaper {
    pub id: Uuid,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub kind: PaperKind,
    pub status: PaperStatus,
    pub is_team_leader: bool,
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaperTeamMemberView {
    pub user_id: UserId,
    pub email: String,
    pub role: GlobalRole,
    pub is_leader: bool,
    pub writer_order: Option<i32>,
    pub created_at: String,
}

#[derive(Debug, Error)]
pub enum V2Error {
    #[error("V2 global role is missing for user {user_id}")]
    RoleMissing { user_id: UserId },
    #[error("user {user_id} with role {actual:?} cannot perform an operation requiring {required}")]
    RoleForbidden {
        user_id: UserId,
        required: &'static str,
        actual: GlobalRole,
    },
    #[error("user {user_id} cannot change role while owning a V2 personal paper")]
    PersonalPaperOwnershipConflict { user_id: UserId },
    #[error("user {user_id} cannot become or lose a role while assigned to a V2 Paper Team")]
    TeamMembershipConflict { user_id: UserId },
    #[error("workspace {workspace_id} is not available as exactly one V2 paper workspace")]
    WorkspaceConflict { workspace_id: WorkspaceId },
    #[error("a live file already uses path {path} in workspace {workspace_id}")]
    DuplicateFilePath {
        workspace_id: WorkspaceId,
        path: String,
    },
    #[error("{entity} already exists")]
    Conflict { entity: &'static str },
    #[error("{entity} was not found")]
    NotFound { entity: &'static str },
    #[error("invalid V2 global role persisted: {value}")]
    InvalidRole { value: String },
    #[error("invalid paper status persisted: {value}")]
    InvalidStatus { value: String },
    #[error("invalid paper path: {message}")]
    InvalidPath { message: String },
    #[error("paper name must contain between 1 and 200 characters")]
    InvalidName,
    #[error("persistent V2 data is invalid: {message}")]
    Integrity { message: String },
    #[error("workspace version conflict: expected {expected}, actual {actual}")]
    VersionConflict { expected: u64, actual: u64 },
    #[error("V2 persistence failed")]
    Database(#[source] sqlx::Error),
}

#[derive(Clone, Debug)]
pub struct V2Repository {
    pub(crate) database: Database,
}

impl V2Repository {
    #[must_use]
    pub const fn new(database: Database) -> Self {
        Self { database }
    }

    pub async fn get_global_role(
        &self,
        user_id: UserId,
    ) -> Result<Option<GlobalRoleAssignment>, V2Error> {
        let row = sqlx::query(
            "SELECT user_id,role,created_at::text AS created_at,updated_at::text AS updated_at \
             FROM latex_core.global_user_roles WHERE user_id=$1",
        )
        .bind(user_id.as_uuid())
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        row.map(decode_role_assignment).transpose()
    }

    pub async fn list_v2_users(&self) -> Result<Vec<V2User>, V2Error> {
        let rows = sqlx::query(
            "SELECT u.id,c.email,c.enabled,c.account_type,c.must_change_password,g.role,u.created_at::text AS created_at,delivery.id AS email_delivery_id,delivery.status AS email_delivery_status,COALESCE(delivery.expires_at<=statement_timestamp(),FALSE) AS email_delivery_expired \
             FROM latex_core.users u \
             JOIN latex_core.user_credentials c ON c.user_id=u.id \
             LEFT JOIN latex_core.global_user_roles g ON g.user_id=u.id \
             LEFT JOIN LATERAL (SELECT id,status,expires_at FROM latex_core.email_outbox WHERE account_user_id=u.id ORDER BY created_at DESC,id DESC LIMIT 1) delivery ON TRUE \
             ORDER BY c.email",
        )
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(decode_v2_user).collect()
    }

    pub async fn set_global_role(
        &self,
        user_id: UserId,
        role: GlobalRole,
    ) -> Result<GlobalRoleAssignment, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_user(&mut tx, user_id).await?;
        validate_role_change(&mut tx, user_id, role).await?;
        let row = sqlx::query(
            "INSERT INTO latex_core.global_user_roles (user_id,role) VALUES ($1,$2) \
             ON CONFLICT (user_id) DO UPDATE SET role=EXCLUDED.role,updated_at=statement_timestamp() \
             RETURNING user_id,role,created_at::text AS created_at,updated_at::text AS updated_at",
        )
        .bind(user_id.as_uuid())
        .bind(role.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        let assignment = decode_role_assignment(row)?;
        revoke_user_sessions(&mut tx, user_id).await?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(assignment)
    }

    pub async fn remove_global_role(&self, user_id: UserId) -> Result<(), V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_user(&mut tx, user_id).await?;
        if owns_personal_paper(&mut tx, user_id).await? {
            return Err(V2Error::PersonalPaperOwnershipConflict { user_id });
        }
        if has_team_membership(&mut tx, user_id).await? {
            return Err(V2Error::TeamMembershipConflict { user_id });
        }
        let result = sqlx::query("DELETE FROM latex_core.global_user_roles WHERE user_id=$1")
            .bind(user_id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(V2Error::Database)?;
        if result.rows_affected() == 0 {
            return Err(V2Error::RoleMissing { user_id });
        }
        revoke_user_sessions(&mut tx, user_id).await?;
        tx.commit().await.map_err(V2Error::Database)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_initialized_personal_paper(
        &self,
        owner_user_id: UserId,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        name: &str,
        main_path: &LogicalPath,
        main_blob_hash: BlobHash,
        main_size_bytes: u64,
    ) -> Result<(PersonalPaper, PaperFile), V2Error> {
        validate_name(name)?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_user(&mut tx, owner_user_id).await?;
        require_role(&mut tx, owner_user_id, &[GlobalRole::Writer], "writer").await?;
        create_initial_workspace(
            &mut tx,
            tenant_id,
            owner_user_id,
            workspace_id,
            main_path,
            main_blob_hash,
            main_size_bytes,
        )
        .await?;
        let paper_row = sqlx::query(
            "INSERT INTO latex_core.personal_papers (id,owner_user_id,workspace_id,name) \
             VALUES ($1,$2,$3,$4) \
             RETURNING id,owner_user_id,workspace_id,name,status,created_at::text AS created_at,updated_at::text AS updated_at",
        )
        .bind(Uuid::new_v4())
        .bind(owner_user_id.as_uuid())
        .bind(workspace_id.as_uuid())
        .bind(name)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_conflict(error, "personal paper"))?;
        let file_row = insert_initial_paper_file(&mut tx, workspace_id, main_path).await?;
        let paper = decode_personal_paper(paper_row)?;
        let file = decode_paper_file(file_row)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok((paper, file))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_initialized_paper_team(
        &self,
        creator: UserId,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        name: &str,
        leader_writer_id: UserId,
        writer_ids: &[UserId],
        mentor_ids: &[UserId],
        main_path: &LogicalPath,
        main_blob_hash: BlobHash,
        main_size_bytes: u64,
    ) -> Result<(PaperTeam, PaperFile), V2Error> {
        validate_name(name)?;
        let leader_role = self
            .get_global_role(leader_writer_id)
            .await?
            .ok_or(V2Error::RoleMissing {
                user_id: leader_writer_id,
            })?
            .role;
        if leader_role != GlobalRole::Writer {
            return Err(V2Error::RoleForbidden {
                user_id: leader_writer_id,
                required: "writer",
                actual: leader_role,
            });
        }
        if writer_ids.is_empty() || !writer_ids.contains(&leader_writer_id) {
            return Err(V2Error::RoleForbidden {
                user_id: leader_writer_id,
                required: "assigned Paper Team Writer Leader",
                actual: leader_role,
            });
        }
        let mut unique = HashSet::new();
        if writer_ids
            .iter()
            .chain(mentor_ids)
            .any(|user_id| !unique.insert(*user_id))
        {
            return Err(V2Error::Conflict {
                entity: "duplicate Paper Team assignment",
            });
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_user(&mut tx, creator).await?;
        require_role(&mut tx, creator, &[GlobalRole::Admin], "admin").await?;
        for writer in writer_ids {
            lock_user(&mut tx, *writer).await?;
            require_role(&mut tx, *writer, &[GlobalRole::Writer], "writer").await?;
        }
        for mentor in mentor_ids {
            lock_user(&mut tx, *mentor).await?;
            require_role(&mut tx, *mentor, &[GlobalRole::Mentor], "mentor").await?;
        }
        create_initial_workspace(
            &mut tx,
            tenant_id,
            creator,
            workspace_id,
            main_path,
            main_blob_hash,
            main_size_bytes,
        )
        .await?;
        let team_row = sqlx::query(
            "INSERT INTO latex_core.paper_teams (id,workspace_id,name,created_by_user_id) \
             VALUES ($1,$2,$3,$4) \
             RETURNING id,workspace_id,name,status,created_by_user_id,created_at::text AS created_at,updated_at::text AS updated_at",
        )
        .bind(Uuid::new_v4())
        .bind(workspace_id.as_uuid())
        .bind(name)
        .bind(creator.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_conflict(error, "Paper Team"))?;
        let team = decode_paper_team(team_row)?;
        for member in writer_ids.iter().chain(mentor_ids) {
            let writer_order = writer_ids
                .iter()
                .position(|writer| writer == member)
                .and_then(|position| i32::try_from(position + 1).ok());
            sqlx::query(
                "INSERT INTO latex_core.paper_team_members \
                 (paper_team_id,user_id,assigned_by_user_id,is_leader,writer_order) VALUES ($1,$2,$3,$4,$5)",
            )
            .bind(team.id)
            .bind(member.as_uuid())
            .bind(creator.as_uuid())
            .bind(*member == leader_writer_id)
            .bind(writer_order)
            .execute(&mut *tx)
            .await
            .map_err(|error| map_conflict(error, "Paper Team membership"))?;
        }
        let file_row = insert_initial_paper_file(&mut tx, workspace_id, main_path).await?;
        let file = decode_paper_file(file_row)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok((team, file))
    }

    pub async fn create_personal_paper(
        &self,
        owner_user_id: UserId,
        workspace_id: WorkspaceId,
        name: &str,
    ) -> Result<PersonalPaper, V2Error> {
        validate_name(name)?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_user(&mut tx, owner_user_id).await?;
        require_role(&mut tx, owner_user_id, &[GlobalRole::Writer], "writer").await?;
        let workspace_owner = lock_workspace(&mut tx, workspace_id).await?;
        if workspace_owner != *owner_user_id.as_uuid()
            || workspace_is_v2_paper(&mut tx, workspace_id).await?
        {
            return Err(V2Error::WorkspaceConflict { workspace_id });
        }
        let row = sqlx::query(
            "INSERT INTO latex_core.personal_papers (id,owner_user_id,workspace_id,name) \
             VALUES ($1,$2,$3,$4) \
             RETURNING id,owner_user_id,workspace_id,name,status,created_at::text AS created_at,updated_at::text AS updated_at",
        )
        .bind(Uuid::new_v4())
        .bind(owner_user_id.as_uuid())
        .bind(workspace_id.as_uuid())
        .bind(name)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_conflict(error, "personal paper"))?;
        let paper = decode_personal_paper(row)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(paper)
    }

    pub async fn personal_paper(&self, id: Uuid) -> Result<PersonalPaper, V2Error> {
        let row = sqlx::query(
            "SELECT id,owner_user_id,workspace_id,name,status,created_at::text AS created_at,updated_at::text AS updated_at \
             FROM latex_core.personal_papers WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound {
            entity: "personal paper",
        })?;
        decode_personal_paper(row)
    }

    pub async fn list_personal_papers(
        &self,
        owner_user_id: UserId,
    ) -> Result<Vec<PersonalPaper>, V2Error> {
        let rows = sqlx::query(
            "SELECT id,owner_user_id,workspace_id,name,status,created_at::text AS created_at,updated_at::text AS updated_at \
             FROM latex_core.personal_papers WHERE owner_user_id=$1 ORDER BY updated_at DESC,id",
        )
        .bind(owner_user_id.as_uuid())
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(decode_personal_paper).collect()
    }

    pub async fn set_personal_paper_status(
        &self,
        id: Uuid,
        status: PaperStatus,
    ) -> Result<PersonalPaper, V2Error> {
        let row = sqlx::query(
            "UPDATE latex_core.personal_papers SET status=$2,updated_at=statement_timestamp() WHERE id=$1 \
             RETURNING id,owner_user_id,workspace_id,name,status,created_at::text AS created_at,updated_at::text AS updated_at",
        )
        .bind(id)
        .bind(status.as_str())
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound {
            entity: "personal paper",
        })?;
        decode_personal_paper(row)
    }

    pub async fn create_paper_team(
        &self,
        created_by_user_id: UserId,
        workspace_id: WorkspaceId,
        name: &str,
        leader_writer_id: UserId,
    ) -> Result<PaperTeam, V2Error> {
        validate_name(name)?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_users(&mut tx, created_by_user_id, leader_writer_id).await?;
        require_role(&mut tx, created_by_user_id, &[GlobalRole::Admin], "admin").await?;
        require_role(&mut tx, leader_writer_id, &[GlobalRole::Writer], "writer").await?;
        lock_workspace(&mut tx, workspace_id).await?;
        if workspace_is_v2_paper(&mut tx, workspace_id).await? {
            return Err(V2Error::WorkspaceConflict { workspace_id });
        }
        let row = sqlx::query(
            "INSERT INTO latex_core.paper_teams (id,workspace_id,name,created_by_user_id) \
             VALUES ($1,$2,$3,$4) \
             RETURNING id,workspace_id,name,status,created_by_user_id,created_at::text AS created_at,updated_at::text AS updated_at",
        )
        .bind(Uuid::new_v4())
        .bind(workspace_id.as_uuid())
        .bind(name)
        .bind(created_by_user_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_conflict(error, "Paper Team"))?;
        let team = decode_paper_team(row)?;
        sqlx::query(
            "INSERT INTO latex_core.paper_team_members \
             (paper_team_id,user_id,assigned_by_user_id,is_leader,writer_order) VALUES ($1,$2,$3,TRUE,1)",
        )
        .bind(team.id)
        .bind(leader_writer_id.as_uuid())
        .bind(created_by_user_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(|error| map_conflict(error, "Paper Team Leader membership"))?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(team)
    }

    pub async fn paper_team(&self, id: Uuid) -> Result<PaperTeam, V2Error> {
        let row = sqlx::query(
            "SELECT id,workspace_id,name,status,created_by_user_id,created_at::text AS created_at,updated_at::text AS updated_at \
             FROM latex_core.paper_teams WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound {
            entity: "Paper Team",
        })?;
        decode_paper_team(row)
    }

    pub async fn set_paper_team_status(
        &self,
        id: Uuid,
        status: PaperStatus,
    ) -> Result<PaperTeam, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_paper_team(&mut tx, id).await?;
        if status == PaperStatus::Active && !active_team_has_one_writer_leader(&mut tx, id).await? {
            return Err(V2Error::Conflict {
                entity: "active Paper Team without exactly one Writer Leader",
            });
        }
        let row = sqlx::query(
            "UPDATE latex_core.paper_teams SET status=$2,updated_at=statement_timestamp() WHERE id=$1 \
             RETURNING id,workspace_id,name,status,created_by_user_id,created_at::text AS created_at,updated_at::text AS updated_at",
        )
        .bind(id)
        .bind(status.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound {
            entity: "Paper Team",
        })?;
        let team = decode_paper_team(row)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(team)
    }

    pub async fn add_paper_team_member(
        &self,
        paper_team_id: Uuid,
        user_id: UserId,
        assigned_by_user_id: UserId,
    ) -> Result<PaperTeamMember, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_users(&mut tx, user_id, assigned_by_user_id).await?;
        require_role(&mut tx, assigned_by_user_id, &[GlobalRole::Admin], "admin").await?;
        let member_role = require_role(
            &mut tx,
            user_id,
            &[GlobalRole::Writer, GlobalRole::Mentor],
            "writer or mentor",
        )
        .await?;
        lock_paper_team(&mut tx, paper_team_id).await?;
        let writer_order = if member_role == GlobalRole::Writer {
            Some(
                sqlx::query_scalar::<_, i32>(
                    "SELECT COALESCE(max(writer_order),0)+1 FROM latex_core.paper_team_members WHERE paper_team_id=$1",
                )
                .bind(paper_team_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(V2Error::Database)?,
            )
        } else {
            None
        };
        let row = sqlx::query(
            "INSERT INTO latex_core.paper_team_members (paper_team_id,user_id,assigned_by_user_id,writer_order) \
             VALUES ($1,$2,$3,$4) \
             RETURNING paper_team_id,user_id,assigned_by_user_id,is_leader,writer_order,created_at::text AS created_at",
        )
        .bind(paper_team_id)
        .bind(user_id.as_uuid())
        .bind(assigned_by_user_id.as_uuid())
        .bind(writer_order)
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_conflict(error, "Paper Team membership"))?;
        let member = decode_paper_team_member(row)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(member)
    }

    pub async fn remove_paper_team_member(
        &self,
        paper_team_id: Uuid,
        user_id: UserId,
        removed_by_user_id: UserId,
    ) -> Result<(), V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_users(&mut tx, user_id, removed_by_user_id).await?;
        require_role(&mut tx, removed_by_user_id, &[GlobalRole::Admin], "admin").await?;
        lock_paper_team(&mut tx, paper_team_id).await?;
        let leader_of_active_team: bool = sqlx::query_scalar(
            "SELECT EXISTS(\
                 SELECT 1 FROM latex_core.paper_team_members m \
                 JOIN latex_core.paper_teams t ON t.id=m.paper_team_id \
                 WHERE m.paper_team_id=$1 AND m.user_id=$2 AND m.is_leader AND t.status='active'\
             )",
        )
        .bind(paper_team_id)
        .bind(user_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        if leader_of_active_team {
            return Err(V2Error::Conflict {
                entity: "active Paper Team Leader membership cannot be removed",
            });
        }
        let result = sqlx::query(
            "DELETE FROM latex_core.paper_team_members WHERE paper_team_id=$1 AND user_id=$2",
        )
        .bind(paper_team_id)
        .bind(user_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        if result.rows_affected() == 0 {
            return Err(V2Error::NotFound {
                entity: "Paper Team membership",
            });
        }
        tx.commit().await.map_err(V2Error::Database)
    }

    pub async fn list_paper_team_members(
        &self,
        paper_team_id: Uuid,
    ) -> Result<Vec<PaperTeamMember>, V2Error> {
        let rows = sqlx::query(
            "SELECT paper_team_id,user_id,assigned_by_user_id,is_leader,writer_order,created_at::text AS created_at \
             FROM latex_core.paper_team_members WHERE paper_team_id=$1 ORDER BY writer_order NULLS LAST,created_at,user_id",
        )
        .bind(paper_team_id)
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(decode_paper_team_member).collect()
    }

    pub async fn change_paper_team_leader(
        &self,
        paper_team_id: Uuid,
        leader_writer_id: UserId,
        changed_by_admin_user_id: UserId,
    ) -> Result<PaperTeamMember, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_users(&mut tx, leader_writer_id, changed_by_admin_user_id).await?;
        require_role(
            &mut tx,
            changed_by_admin_user_id,
            &[GlobalRole::Admin],
            "admin",
        )
        .await?;
        require_role(&mut tx, leader_writer_id, &[GlobalRole::Writer], "writer").await?;
        lock_paper_team(&mut tx, paper_team_id).await?;
        let member_exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM latex_core.paper_team_members \
             WHERE paper_team_id=$1 AND user_id=$2)",
        )
        .bind(paper_team_id)
        .bind(leader_writer_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        if !member_exists {
            return Err(V2Error::RoleForbidden {
                user_id: leader_writer_id,
                required: "assigned Paper Team Writer Leader",
                actual: GlobalRole::Writer,
            });
        }
        sqlx::query(
            "UPDATE latex_core.paper_team_members SET is_leader=FALSE \
             WHERE paper_team_id=$1 AND is_leader",
        )
        .bind(paper_team_id)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        let row = sqlx::query(
            "UPDATE latex_core.paper_team_members SET is_leader=TRUE \
             WHERE paper_team_id=$1 AND user_id=$2 \
             RETURNING paper_team_id,user_id,assigned_by_user_id,is_leader,writer_order,created_at::text AS created_at",
        )
        .bind(paper_team_id)
        .bind(leader_writer_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        let leader = decode_paper_team_member(row)?;
        tx.commit().await.map_err(V2Error::Database)?;
        tracing::info!(
            %paper_team_id,
            %leader_writer_id,
            %changed_by_admin_user_id,
            "Paper Team Writer Leader reassigned"
        );
        Ok(leader)
    }

    pub async fn list_paper_teams(&self) -> Result<Vec<PaperTeam>, V2Error> {
        let rows = sqlx::query(
            "SELECT id,workspace_id,name,status,created_by_user_id,created_at::text AS created_at,updated_at::text AS updated_at \
             FROM latex_core.paper_teams ORDER BY updated_at DESC,id",
        )
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(decode_paper_team).collect()
    }

    pub async fn list_paper_team_member_views(
        &self,
        paper_team_id: Uuid,
    ) -> Result<Vec<PaperTeamMemberView>, V2Error> {
        let rows = sqlx::query(
            "SELECT m.user_id,c.email,g.role,m.is_leader,m.writer_order,m.created_at::text AS created_at \
             FROM latex_core.paper_team_members m \
             JOIN latex_core.user_credentials c ON c.user_id=m.user_id \
             JOIN latex_core.global_user_roles g ON g.user_id=m.user_id \
             WHERE m.paper_team_id=$1 ORDER BY m.writer_order NULLS LAST,g.role,c.email",
        )
        .bind(paper_team_id)
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(decode_team_member_view).collect()
    }

    pub async fn writer_papers(&self, writer: UserId) -> Result<Vec<WriterPaper>, V2Error> {
        let role = self
            .get_global_role(writer)
            .await?
            .ok_or(V2Error::RoleMissing { user_id: writer })?
            .role;
        if role != GlobalRole::Writer {
            return Err(V2Error::RoleForbidden {
                user_id: writer,
                required: "writer",
                actual: role,
            });
        }
        let rows = sqlx::query(
            "SELECT id,workspace_id,name,'personal' AS kind,status,FALSE AS is_team_leader,updated_at::text AS updated_at \
             FROM latex_core.personal_papers WHERE owner_user_id=$1 \
             UNION ALL \
             SELECT t.id,t.workspace_id,t.name,'team' AS kind,t.status,m.is_leader AS is_team_leader,t.updated_at::text AS updated_at \
             FROM latex_core.paper_teams t \
             JOIN latex_core.paper_team_members m ON m.paper_team_id=t.id \
             WHERE m.user_id=$1 ORDER BY updated_at DESC,id",
        )
        .bind(writer.as_uuid())
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(decode_writer_paper).collect()
    }

    pub async fn writer_paper(
        &self,
        writer: UserId,
        paper_id: Uuid,
    ) -> Result<WriterPaper, V2Error> {
        self.writer_papers(writer)
            .await?
            .into_iter()
            .find(|paper| paper.id == paper_id)
            .ok_or(V2Error::NotFound { entity: "paper" })
    }

    pub async fn list_live_paper_files(
        &self,
        workspace_id: WorkspaceId,
    ) -> Result<Vec<PaperFile>, V2Error> {
        let rows = sqlx::query(
            "SELECT file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at \
             FROM latex_core.paper_files WHERE workspace_id=$1 AND NOT tombstoned ORDER BY path",
        )
        .bind(workspace_id.as_uuid())
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        rows.into_iter().map(decode_paper_file).collect()
    }

    pub async fn collaboration_access(
        &self,
        user_id: UserId,
        paper_id: Uuid,
        file_id: Uuid,
    ) -> Result<CollaborationAccess, V2Error> {
        let role = self
            .get_global_role(user_id)
            .await?
            .ok_or(V2Error::RoleMissing { user_id })?
            .role;
        if role == GlobalRole::Admin {
            return Err(V2Error::RoleForbidden {
                user_id,
                required: "paper collaboration participant",
                actual: role,
            });
        }
        let rows = sqlx::query(
            "SELECT workspace_id,status,'personal' AS kind,(owner_user_id=$2) AS assigned \
             FROM latex_core.personal_papers WHERE id=$1 \
             UNION ALL \
             SELECT t.workspace_id,t.status,'team' AS kind,EXISTS( \
                 SELECT 1 FROM latex_core.paper_team_members m \
                 WHERE m.paper_team_id=t.id AND m.user_id=$2) AS assigned \
             FROM latex_core.paper_teams t WHERE t.id=$1",
        )
        .bind(paper_id)
        .bind(user_id.as_uuid())
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        if rows.len() != 1 {
            return Err(V2Error::NotFound { entity: "paper" });
        }
        let row = &rows[0];
        let workspace_id =
            WorkspaceId::from_uuid(row.try_get("workspace_id").map_err(V2Error::Database)?);
        let kind: String = row.try_get("kind").map_err(V2Error::Database)?;
        let status: String = row.try_get("status").map_err(V2Error::Database)?;
        let assigned: bool = row.try_get("assigned").map_err(V2Error::Database)?;
        let permitted = assigned
            && matches!(
                (kind.as_str(), role),
                ("personal", GlobalRole::Writer)
                    | ("team", GlobalRole::Writer | GlobalRole::Mentor)
            );
        if !permitted {
            return Err(V2Error::RoleForbidden {
                user_id,
                required: "assigned paper collaborator",
                actual: role,
            });
        }
        let file = self.paper_file(file_id).await?;
        if file.workspace_id != workspace_id || file.tombstoned {
            return Err(V2Error::NotFound {
                entity: "live paper file",
            });
        }
        let policy = self.file_policy(file_id).await?;
        if !policy.visible_to_participants() {
            return Err(V2Error::NotFound {
                entity: "live paper file",
            });
        }
        let epoch = match sqlx::query_scalar::<_, i64>(
            "SELECT document_epoch FROM latex_core.paper_collaboration_state WHERE workspace_id=$1",
        )
        .bind(workspace_id.as_uuid())
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?
        {
            Some(epoch) => epoch,
            None => sqlx::query_scalar(
                "INSERT INTO latex_core.paper_collaboration_state (workspace_id) VALUES ($1) \
                 ON CONFLICT (workspace_id) DO UPDATE SET workspace_id=EXCLUDED.workspace_id \
                 RETURNING document_epoch",
            )
            .bind(workspace_id.as_uuid())
            .fetch_one(self.database.pool())
            .await
            .map_err(V2Error::Database)?,
        };
        let document_epoch = u64::try_from(epoch).map_err(|_| V2Error::Integrity {
            message: "negative collaboration document epoch".to_owned(),
        })?;
        let mode = if role == GlobalRole::Writer
            && status == PaperStatus::Active.as_str()
            && policy.content_editable()
        {
            CollaborationAccessMode::ReadWrite
        } else {
            CollaborationAccessMode::ReadOnly
        };
        Ok(CollaborationAccess {
            workspace_id,
            file,
            document_epoch,
            mode,
        })
    }

    pub async fn collaboration_recovery(
        &self,
        workspace_id: WorkspaceId,
        file_id: Uuid,
        document_epoch: u64,
    ) -> Result<CollaborationRecovery, V2Error> {
        let epoch = i64::try_from(document_epoch).map_err(|_| V2Error::Integrity {
            message: "collaboration epoch exceeds PostgreSQL BIGINT".to_owned(),
        })?;
        let snapshot_row = sqlx::query(
            "SELECT through_sequence,compressed_state FROM latex_core.collaboration_snapshots \
             WHERE workspace_id=$1 AND file_id=$2 AND document_epoch=$3 \
             ORDER BY through_sequence DESC LIMIT 1",
        )
        .bind(workspace_id.as_uuid())
        .bind(file_id)
        .bind(epoch)
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        let snapshot = snapshot_row
            .map(|row| {
                let sequence: i64 = row.try_get("through_sequence").map_err(V2Error::Database)?;
                Ok((
                    u64::try_from(sequence).map_err(|_| V2Error::Integrity {
                        message: "negative collaboration snapshot sequence".to_owned(),
                    })?,
                    row.try_get("compressed_state").map_err(V2Error::Database)?,
                ))
            })
            .transpose()?;
        let after = snapshot.as_ref().map_or(0, |(sequence, _)| *sequence);
        let after = i64::try_from(after).map_err(|_| V2Error::Integrity {
            message: "collaboration sequence exceeds PostgreSQL BIGINT".to_owned(),
        })?;
        let rows = sqlx::query(
            "SELECT id,update_bytes FROM latex_core.collaboration_updates \
             WHERE workspace_id=$1 AND file_id=$2 AND document_epoch=$3 AND id>$4 ORDER BY id",
        )
        .bind(workspace_id.as_uuid())
        .bind(file_id)
        .bind(epoch)
        .bind(after)
        .fetch_all(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        let updates = rows
            .into_iter()
            .map(|row| {
                let sequence: i64 = row.try_get("id").map_err(V2Error::Database)?;
                Ok((
                    u64::try_from(sequence).map_err(|_| V2Error::Integrity {
                        message: "negative collaboration update sequence".to_owned(),
                    })?,
                    row.try_get("update_bytes").map_err(V2Error::Database)?,
                ))
            })
            .collect::<Result<Vec<_>, V2Error>>()?;
        Ok(CollaborationRecovery {
            document_epoch,
            snapshot,
            updates,
        })
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub async fn persist_collaboration_batch(
        &self,
        workspace_id: WorkspaceId,
        file_id: Uuid,
        document_epoch: u64,
        updates: &[CollaborationUpdateInput],
        blob_hash: BlobHash,
        size_bytes: u64,
    ) -> Result<u64, V2Error> {
        if updates.is_empty() {
            return Err(V2Error::Integrity {
                message: "empty collaboration durability batch".to_owned(),
            });
        }
        let materialization_actor = updates
            .last()
            .map(|update| update.actor_user_id)
            .ok_or_else(|| V2Error::Integrity {
                message: "empty collaboration durability batch".to_owned(),
            })?;
        let epoch = i64::try_from(document_epoch).map_err(|_| V2Error::Integrity {
            message: "collaboration epoch exceeds PostgreSQL BIGINT".to_owned(),
        })?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        workspace_mutation_lock(&mut tx, workspace_id).await?;
        let current = lock_live_file(&mut tx, file_id).await?;
        if current.workspace_id != workspace_id {
            return Err(V2Error::NotFound {
                entity: "live paper file",
            });
        }
        assert_content_policy(&mut tx, file_id).await?;
        let current_epoch: i64 = sqlx::query_scalar(
            "SELECT document_epoch FROM latex_core.paper_collaboration_state \
             WHERE workspace_id=$1 FOR UPDATE",
        )
        .bind(workspace_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound {
            entity: "collaboration state",
        })?;
        if current_epoch != epoch {
            return Err(V2Error::Conflict {
                entity: "collaboration document epoch",
            });
        }
        let mut durable_sequence = 0_u64;
        for update in updates {
            let sequence: i64 = sqlx::query_scalar(
                "INSERT INTO latex_core.collaboration_updates \
                 (workspace_id,file_id,document_epoch,actor_user_id,update_bytes) \
                 VALUES ($1,$2,$3,$4,$5) RETURNING id",
            )
            .bind(workspace_id.as_uuid())
            .bind(file_id)
            .bind(epoch)
            .bind(update.actor_user_id.as_uuid())
            .bind(&update.update_bytes)
            .fetch_one(&mut *tx)
            .await
            .map_err(V2Error::Database)?;
            durable_sequence = u64::try_from(sequence).map_err(|_| V2Error::Integrity {
                message: "negative collaboration update sequence".to_owned(),
            })?;
        }
        let version: i64 = sqlx::query_scalar(
            "SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1 FOR UPDATE",
        )
        .bind(workspace_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        let version = u64::try_from(version).map_err(|_| V2Error::Integrity {
            message: "negative workspace version".to_owned(),
        })?;
        append_workspace_operation(
            &mut tx,
            workspace_id,
            materialization_actor,
            version,
            json!({"op":"put_file","path":current.path,"blob_hash":blob_hash,"size_bytes":size_bytes}),
        )
        .await?;
        sqlx::query(
            "UPDATE latex_core.paper_files SET revision=revision+1,updated_at=statement_timestamp() \
             WHERE file_id=$1",
        )
        .bind(file_id)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        sqlx::query(
            "UPDATE latex_core.paper_collaboration_state SET updated_at=statement_timestamp() \
             WHERE workspace_id=$1",
        )
        .bind(workspace_id.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(durable_sequence)
    }

    pub async fn save_collaboration_snapshot(
        &self,
        workspace_id: WorkspaceId,
        file_id: Uuid,
        document_epoch: u64,
        through_sequence: u64,
        compressed_state: &[u8],
    ) -> Result<(), V2Error> {
        let epoch = i64::try_from(document_epoch).map_err(|_| V2Error::Integrity {
            message: "collaboration epoch exceeds PostgreSQL BIGINT".to_owned(),
        })?;
        let sequence = i64::try_from(through_sequence).map_err(|_| V2Error::Integrity {
            message: "collaboration sequence exceeds PostgreSQL BIGINT".to_owned(),
        })?;
        sqlx::query(
            "INSERT INTO latex_core.collaboration_snapshots \
             (workspace_id,file_id,document_epoch,through_sequence,compressed_state) \
             VALUES ($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING",
        )
        .bind(workspace_id.as_uuid())
        .bind(file_id)
        .bind(epoch)
        .bind(sequence)
        .bind(compressed_state)
        .execute(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn create_file_with_event(
        &self,
        workspace_id: WorkspaceId,
        actor: UserId,
        expected_version: u64,
        path: LogicalPath,
        blob_hash: BlobHash,
        size_bytes: u64,
    ) -> Result<(PaperFile, u64), V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        workspace_mutation_lock(&mut tx, workspace_id).await?;
        require_writer_workspace_access(&mut tx, actor, workspace_id, true).await?;
        let row = sqlx::query(
            "INSERT INTO latex_core.paper_files (file_id,workspace_id,path) VALUES ($1,$2,$3) \
             RETURNING file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at",
        )
        .bind(Uuid::new_v4())
        .bind(workspace_id.as_uuid())
        .bind(path.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_file_conflict(error, workspace_id, &path))?;
        let version = append_workspace_operation(
            &mut tx,
            workspace_id,
            actor,
            expected_version,
            json!({"op":"put_file","path":path,"blob_hash":blob_hash,"size_bytes":size_bytes}),
        )
        .await?;
        let file = decode_paper_file(row)?;
        record_structural_operation(
            &mut tx,
            workspace_id,
            actor,
            "CREATE_FILE",
            Some(file.file_id),
            json!({"op":"create_file","file_id":file.file_id,"path":file.path,"blob_hash":blob_hash,"size_bytes":size_bytes,"expected_revision":file.revision}),
            json!({"op":"delete_file","file_id":file.file_id,"path":file.path,"expected_revision":file.revision}),
            expected_version,
            version,
        )
        .await?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok((file, version))
    }

    pub async fn save_file_with_event(
        &self,
        file_id: Uuid,
        actor: UserId,
        expected_version: u64,
        blob_hash: BlobHash,
        size_bytes: u64,
    ) -> Result<(PaperFile, u64), V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        let workspace_id = file_workspace(&mut tx, file_id).await?;
        workspace_mutation_lock(&mut tx, workspace_id).await?;
        let current = lock_live_file(&mut tx, file_id).await?;
        require_writer_workspace_access(&mut tx, actor, current.workspace_id, true).await?;
        assert_content_policy(&mut tx, file_id).await?;
        let version = append_workspace_operation(
            &mut tx,
            current.workspace_id,
            actor,
            expected_version,
            json!({"op":"put_file","path":current.path,"blob_hash":blob_hash,"size_bytes":size_bytes}),
        )
        .await?;
        let row = sqlx::query(
            "UPDATE latex_core.paper_files SET revision=revision+1,updated_at=statement_timestamp() \
             WHERE file_id=$1 RETURNING file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at",
        )
        .bind(file_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        let file = decode_paper_file(row)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok((file, version))
    }

    pub async fn rename_file_with_event(
        &self,
        file_id: Uuid,
        actor: UserId,
        expected_version: u64,
        path: LogicalPath,
    ) -> Result<(PaperFile, u64), V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        let workspace_id = file_workspace(&mut tx, file_id).await?;
        workspace_mutation_lock(&mut tx, workspace_id).await?;
        let current = lock_live_file(&mut tx, file_id).await?;
        require_writer_workspace_access(&mut tx, actor, current.workspace_id, true).await?;
        assert_structure_policy(&mut tx, file_id).await?;
        let old_path = current.path.clone();
        let version = append_workspace_operation(
            &mut tx,
            current.workspace_id,
            actor,
            expected_version,
            json!({"op":"rename_file","from":current.path,"to":path}),
        )
        .await?;
        let row = sqlx::query(
            "UPDATE latex_core.paper_files SET path=$2,revision=revision+1,updated_at=statement_timestamp() \
             WHERE file_id=$1 RETURNING file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at",
        )
        .bind(file_id)
        .bind(path.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_file_conflict(error, current.workspace_id, &path))?;
        let file = decode_paper_file(row)?;
        let old_parent = old_path.as_str().rsplit_once('/').map(|(parent, _)| parent);
        let new_parent = file
            .path
            .as_str()
            .rsplit_once('/')
            .map(|(parent, _)| parent);
        let operation_type = if old_parent == new_parent {
            "RENAME_FILE"
        } else {
            "MOVE_FILE"
        };
        record_structural_operation(
            &mut tx,
            current.workspace_id,
            actor,
            operation_type,
            Some(file.file_id),
            json!({"op":"rename_file","file_id":file.file_id,"from":old_path,"to":file.path,"expected_revision":file.revision}),
            json!({"op":"rename_file","file_id":file.file_id,"from":file.path,"to":old_path,"expected_revision":file.revision}),
            expected_version,
            version,
        )
        .await?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok((file, version))
    }

    pub async fn delete_file_with_event(
        &self,
        file_id: Uuid,
        actor: UserId,
        expected_version: u64,
        blob_hash: BlobHash,
        size_bytes: u64,
        was_main: bool,
    ) -> Result<u64, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        let workspace_id = file_workspace(&mut tx, file_id).await?;
        workspace_mutation_lock(&mut tx, workspace_id).await?;
        let current = lock_live_file(&mut tx, file_id).await?;
        require_writer_workspace_access(&mut tx, actor, current.workspace_id, true).await?;
        assert_structure_policy(&mut tx, file_id).await?;
        let version = append_workspace_operation(
            &mut tx,
            current.workspace_id,
            actor,
            expected_version,
            json!({"op":"delete_file","path":current.path}),
        )
        .await?;
        sqlx::query(
            "UPDATE latex_core.paper_files SET tombstoned=TRUE,tombstoned_at=statement_timestamp(),revision=revision+1,updated_at=statement_timestamp() WHERE file_id=$1",
        )
        .bind(file_id)
        .execute(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        record_structural_operation(
            &mut tx,
            current.workspace_id,
            actor,
            "DELETE_FILE",
            Some(file_id),
            json!({"op":"delete_file","file_id":file_id,"path":current.path,"expected_revision":current.revision + 1}),
            json!({"op":"restore_file","file_id":file_id,"path":current.path,"blob_hash":blob_hash,"size_bytes":size_bytes,"was_main":was_main,"expected_revision":current.revision + 1}),
            expected_version,
            version,
        )
        .await?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(version)
    }

    pub async fn set_main_with_event(
        &self,
        file_id: Uuid,
        actor: UserId,
        expected_version: u64,
        previous_main: Option<LogicalPath>,
    ) -> Result<u64, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        let workspace_id = file_workspace(&mut tx, file_id).await?;
        workspace_mutation_lock(&mut tx, workspace_id).await?;
        let current = lock_live_file(&mut tx, file_id).await?;
        require_writer_workspace_access(&mut tx, actor, current.workspace_id, true).await?;
        assert_main_policy(&mut tx, file_id).await?;
        let version = append_workspace_operation(
            &mut tx,
            current.workspace_id,
            actor,
            expected_version,
            json!({"op":"set_main_file","path":current.path}),
        )
        .await?;
        if let Some(previous_main) = previous_main {
            record_structural_operation(
                &mut tx,
                current.workspace_id,
                actor,
                "SET_MAIN",
                Some(file_id),
                json!({"op":"set_main","from":previous_main,"to":current.path}),
                json!({"op":"set_main","from":current.path,"to":previous_main}),
                expected_version,
                version,
            )
            .await?;
        }
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(version)
    }

    pub async fn structural_undo(
        &self,
        paper_id: Uuid,
        workspace_id: WorkspaceId,
        actor: UserId,
    ) -> Result<StructuralOperationResult, V2Error> {
        self.apply_structural_history(paper_id, workspace_id, actor, false)
            .await
    }

    pub async fn structural_redo(
        &self,
        paper_id: Uuid,
        workspace_id: WorkspaceId,
        actor: UserId,
    ) -> Result<StructuralOperationResult, V2Error> {
        self.apply_structural_history(paper_id, workspace_id, actor, true)
            .await
    }

    #[allow(
        clippy::too_many_lines,
        reason = "the locked safety checks and inverse mutation remain adjacent in one transaction"
    )]
    async fn apply_structural_history(
        &self,
        paper_id: Uuid,
        workspace_id: WorkspaceId,
        actor: UserId,
        redo: bool,
    ) -> Result<StructuralOperationResult, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        workspace_mutation_lock(&mut tx, workspace_id).await?;
        require_writer_workspace_access(&mut tx, actor, workspace_id, true).await?;
        let wanted_state = if redo { "UNDONE" } else { "APPLIED" };
        let history_order = if redo {
            "undone_at DESC,id DESC"
        } else {
            "workspace_version_before DESC,id DESC"
        };
        let history_query = format!(
            "SELECT id,operation_type,file_id,forward_payload,inverse_payload,workspace_version_after \
             FROM latex_core.reversible_structural_operations \
             WHERE paper_id=$1 AND workspace_id=$2 AND actor_user_id=$3 AND state=$4 \
             ORDER BY {history_order} LIMIT 1 FOR UPDATE"
        );
        let row = sqlx::query(&history_query)
            .bind(paper_id)
            .bind(workspace_id.as_uuid())
            .bind(actor.as_uuid())
            .bind(wanted_state)
            .fetch_optional(&mut *tx)
            .await
            .map_err(V2Error::Database)?
            .ok_or(V2Error::Conflict {
                entity: if redo {
                    "structural redo opportunity"
                } else {
                    "structural undo opportunity"
                },
            })?;
        let operation_id: Uuid = row.try_get("id").map_err(V2Error::Database)?;
        let operation_type: String = row.try_get("operation_type").map_err(V2Error::Database)?;
        let file_id: Option<Uuid> = row.try_get("file_id").map_err(V2Error::Database)?;
        let payload: serde_json::Value = row
            .try_get(if redo {
                "forward_payload"
            } else {
                "inverse_payload"
            })
            .map_err(V2Error::Database)?;
        if let Some(file_id) = file_id {
            if operation_type == "SET_MAIN" {
                assert_main_policy(&mut tx, file_id).await?;
            } else {
                assert_structure_policy(&mut tx, file_id).await?;
            }
        }
        let latest_structural: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM latex_core.reversible_structural_operations \
             WHERE workspace_id=$1 AND state='APPLIED' ORDER BY workspace_version_before DESC,id DESC LIMIT 1 FOR UPDATE",
        )
        .bind(workspace_id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?;
        if !redo && latest_structural != Some(operation_id) {
            return Err(V2Error::Conflict {
                entity: "later structural change prevents undo",
            });
        }
        let expected_version = current_workspace_version(&mut tx, workspace_id).await?;
        let operations = apply_structural_payload(&mut tx, workspace_id, &payload).await?;
        if let Some(file_id) = file_id {
            let revision: i64 =
                sqlx::query_scalar("SELECT revision FROM latex_core.paper_files WHERE file_id=$1")
                    .bind(file_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(V2Error::Database)?;
            let payload_column = if redo {
                "inverse_payload"
            } else {
                "forward_payload"
            };
            let revision_query = format!(
                "UPDATE latex_core.reversible_structural_operations \
                 SET {payload_column}=jsonb_set({payload_column}, '{{expected_revision}}', to_jsonb($2::bigint), true) WHERE id=$1"
            );
            sqlx::query(&revision_query)
                .bind(operation_id)
                .bind(revision)
                .execute(&mut *tx)
                .await
                .map_err(V2Error::Database)?;
        }
        let version =
            append_workspace_operations(&mut tx, workspace_id, actor, expected_version, operations)
                .await?;
        let state = if redo { "APPLIED" } else { "UNDONE" };
        let query = if redo {
            "UPDATE latex_core.reversible_structural_operations SET state=$2,undone_at=NULL,redone_at=statement_timestamp() WHERE id=$1"
        } else {
            "UPDATE latex_core.reversible_structural_operations SET state=$2,undone_at=statement_timestamp() WHERE id=$1"
        };
        sqlx::query(query)
            .bind(operation_id)
            .bind(state)
            .execute(&mut *tx)
            .await
            .map_err(V2Error::Database)?;
        tx.commit().await.map_err(V2Error::Database)?;
        tracing::info!(%paper_id, %workspace_id, %operation_id, actor_user_id=%actor, %operation_type, %state, workspace_version=version, "Writer structural history transition committed");
        Ok(StructuralOperationResult {
            operation_id,
            operation_type,
            file_id,
            version,
            state: state.to_owned(),
        })
    }

    pub async fn register_paper_file(
        &self,
        workspace_id: WorkspaceId,
        path: LogicalPath,
    ) -> Result<PaperFile, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        lock_workspace(&mut tx, workspace_id).await?;
        require_v2_paper_workspace(&mut tx, workspace_id).await?;
        let row = sqlx::query(
            "INSERT INTO latex_core.paper_files (file_id,workspace_id,path) VALUES ($1,$2,$3) \
             RETURNING file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at",
        )
        .bind(Uuid::new_v4())
        .bind(workspace_id.as_uuid())
        .bind(path.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_file_conflict(error, workspace_id, &path))?;
        let file = decode_paper_file(row)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(file)
    }

    pub async fn paper_file(&self, file_id: Uuid) -> Result<PaperFile, V2Error> {
        let row = sqlx::query(
            "SELECT file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at \
             FROM latex_core.paper_files WHERE file_id=$1",
        )
        .bind(file_id)
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound {
            entity: "paper file",
        })?;
        decode_paper_file(row)
    }

    pub async fn resolve_live_paper_file(
        &self,
        workspace_id: WorkspaceId,
        path: &LogicalPath,
    ) -> Result<Option<PaperFile>, V2Error> {
        let row = sqlx::query(
            "SELECT file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at \
             FROM latex_core.paper_files WHERE workspace_id=$1 AND path=$2 AND NOT tombstoned",
        )
        .bind(workspace_id.as_uuid())
        .bind(path.as_str())
        .fetch_optional(self.database.pool())
        .await
        .map_err(V2Error::Database)?;
        row.map(decode_paper_file).transpose()
    }

    pub async fn rename_paper_file(
        &self,
        file_id: Uuid,
        path: LogicalPath,
    ) -> Result<PaperFile, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        let workspace_id = file_workspace(&mut tx, file_id).await?;
        lock_workspace(&mut tx, workspace_id).await?;
        let tombstoned: bool = sqlx::query_scalar(
            "SELECT tombstoned FROM latex_core.paper_files WHERE file_id=$1 FOR UPDATE",
        )
        .bind(file_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound {
            entity: "paper file",
        })?;
        if tombstoned {
            return Err(V2Error::Conflict {
                entity: "tombstoned paper file",
            });
        }
        let row = sqlx::query(
            "UPDATE latex_core.paper_files \
             SET path=$2,revision=revision+1,updated_at=statement_timestamp() WHERE file_id=$1 \
             RETURNING file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at",
        )
        .bind(file_id)
        .bind(path.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(|error| map_file_conflict(error, workspace_id, &path))?;
        let file = decode_paper_file(row)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(file)
    }

    pub async fn tombstone_paper_file(&self, file_id: Uuid) -> Result<PaperFile, V2Error> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(V2Error::Database)?;
        let workspace_id = file_workspace(&mut tx, file_id).await?;
        lock_workspace(&mut tx, workspace_id).await?;
        let row = sqlx::query(
            "UPDATE latex_core.paper_files \
             SET tombstoned=TRUE,tombstoned_at=statement_timestamp(),revision=revision+1,updated_at=statement_timestamp() \
             WHERE file_id=$1 AND NOT tombstoned \
             RETURNING file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at",
        )
        .bind(file_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(V2Error::Database)?
        .ok_or(V2Error::NotFound {
            entity: "live paper file",
        })?;
        let file = decode_paper_file(row)?;
        tx.commit().await.map_err(V2Error::Database)?;
        Ok(file)
    }
}

async fn create_initial_workspace(
    tx: &mut Transaction<'_, Postgres>,
    tenant_id: TenantId,
    owner: UserId,
    workspace_id: WorkspaceId,
    main_path: &LogicalPath,
    blob_hash: BlobHash,
    size_bytes: u64,
) -> Result<(), V2Error> {
    sqlx::query("INSERT INTO latex_core.workspaces (id,tenant_id,owner_user_id) VALUES ($1,$2,$3)")
        .bind(workspace_id.as_uuid())
        .bind(tenant_id.as_uuid())
        .bind(owner.as_uuid())
        .execute(&mut **tx)
        .await
        .map_err(V2Error::Database)?;
    sqlx::query(
        "INSERT INTO latex_core.workspace_heads (workspace_id,durable_version) VALUES ($1,1)",
    )
    .bind(workspace_id.as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    let payload = json!({
        "schema_version": 1,
        "operations": [
            {"op":"put_file","path":main_path,"blob_hash":blob_hash,"size_bytes":size_bytes},
            {"op":"set_main_file","path":main_path}
        ]
    });
    sqlx::query(
        "INSERT INTO latex_core.workspace_events \
         (workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) \
         VALUES ($1,1,$2,0,'workspace.mutation',1,$3,$4)",
    )
    .bind(workspace_id.as_uuid())
    .bind(Uuid::new_v4())
    .bind(payload)
    .bind(owner.as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    Ok(())
}

async fn insert_initial_paper_file(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
    path: &LogicalPath,
) -> Result<PgRow, V2Error> {
    sqlx::query(
        "INSERT INTO latex_core.paper_files (file_id,workspace_id,path) VALUES ($1,$2,$3) \
         RETURNING file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at",
    )
    .bind(Uuid::new_v4())
    .bind(workspace_id.as_uuid())
    .bind(path.as_str())
    .fetch_one(&mut **tx)
    .await
    .map_err(|error| map_file_conflict(error, workspace_id, path))
}

async fn lock_live_file(
    tx: &mut Transaction<'_, Postgres>,
    file_id: Uuid,
) -> Result<PaperFile, V2Error> {
    let row = sqlx::query(
        "SELECT file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at \
         FROM latex_core.paper_files WHERE file_id=$1 FOR UPDATE",
    )
    .bind(file_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound { entity: "paper file" })?;
    let file = decode_paper_file(row)?;
    if file.tombstoned {
        Err(V2Error::NotFound {
            entity: "live paper file",
        })
    } else {
        Ok(file)
    }
}

async fn require_writer_workspace_access(
    tx: &mut Transaction<'_, Postgres>,
    actor: UserId,
    workspace_id: WorkspaceId,
    require_active: bool,
) -> Result<(), V2Error> {
    require_role(tx, actor, &[GlobalRole::Writer], "writer").await?;
    let status = sqlx::query_scalar::<_, String>(
        "SELECT status FROM latex_core.personal_papers \
         WHERE workspace_id=$1 AND owner_user_id=$2 \
         UNION ALL \
         SELECT t.status FROM latex_core.paper_teams t \
         JOIN latex_core.paper_team_members m ON m.paper_team_id=t.id \
         WHERE t.workspace_id=$1 AND m.user_id=$2",
    )
    .bind(workspace_id.as_uuid())
    .bind(actor.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound { entity: "paper" })?;
    let status = PaperStatus::from_str(&status)?;
    if require_active && status != PaperStatus::Active {
        return Err(V2Error::Conflict {
            entity: "read-only paper",
        });
    }
    Ok(())
}

async fn append_workspace_operation(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
    actor: UserId,
    expected_version: u64,
    operation: serde_json::Value,
) -> Result<u64, V2Error> {
    append_workspace_operations(tx, workspace_id, actor, expected_version, vec![operation]).await
}

async fn append_workspace_operations(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
    actor: UserId,
    expected_version: u64,
    operations: Vec<serde_json::Value>,
) -> Result<u64, V2Error> {
    let expected = i64::try_from(expected_version).map_err(|_| V2Error::Integrity {
        message: "workspace version exceeds PostgreSQL BIGINT".to_owned(),
    })?;
    let actual: i64 = sqlx::query_scalar(
        "SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1 FOR UPDATE",
    )
    .bind(workspace_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound {
        entity: "workspace",
    })?;
    let actual_u64 = u64::try_from(actual).map_err(|_| V2Error::Integrity {
        message: "negative workspace version".to_owned(),
    })?;
    if actual != expected {
        return Err(V2Error::VersionConflict {
            expected: expected_version,
            actual: actual_u64,
        });
    }
    let next = actual.checked_add(1).ok_or_else(|| V2Error::Integrity {
        message: "workspace version overflow".to_owned(),
    })?;
    let payload = json!({"schema_version":1,"operations":operations});
    sqlx::query(
        "INSERT INTO latex_core.workspace_events \
         (workspace_id,sequence,event_id,base_version,event_type,event_schema_version,payload,created_by_user_id) \
         VALUES ($1,$2,$3,$4,'workspace.mutation',1,$5,$6)",
    )
    .bind(workspace_id.as_uuid())
    .bind(next)
    .bind(Uuid::new_v4())
    .bind(actual)
    .bind(payload)
    .bind(actor.as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    sqlx::query(
        "UPDATE latex_core.workspace_heads SET durable_version=$2,updated_at=statement_timestamp() WHERE workspace_id=$1",
    )
    .bind(workspace_id.as_uuid())
    .bind(next)
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    // A durable canonical edit immediately invalidates any in-flight exact
    // hash, even before the browser's two-second debounce submits the next
    // snapshot. This closes the stale-promotion window during that debounce.
    sqlx::query(
        "UPDATE latex_core.v2_paper_build_state SET desired_state_hash=NULL,desired_source_sequence=$2, \
         pending_snapshot_id=NULL,pending_manifest=NULL,pending_state_hash=NULL,pending_source_sequence=NULL, \
         pending_document_epoch=NULL,pending_tenant_id=NULL,pending_user_id=NULL,pending_trigger_type=NULL, \
         pending_compile_key=NULL,pending_engine=NULL,pending_tex_environment_id=NULL,pending_latexmk_profile=NULL, \
         pending_shell_policy=NULL,pending_synctex=NULL,updated_at=statement_timestamp() WHERE workspace_id=$1",
    )
    .bind(workspace_id.as_uuid())
    .bind(next)
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    sqlx::query(
        "UPDATE latex_core.personal_papers SET updated_at=statement_timestamp() WHERE workspace_id=$1",
    )
    .bind(workspace_id.as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    sqlx::query(
        "UPDATE latex_core.paper_teams SET updated_at=statement_timestamp() WHERE workspace_id=$1",
    )
    .bind(workspace_id.as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    u64::try_from(next).map_err(|_| V2Error::Integrity {
        message: "negative workspace version".to_owned(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn record_structural_operation(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
    actor: UserId,
    operation_type: &str,
    file_id: Option<Uuid>,
    forward_payload: serde_json::Value,
    inverse_payload: serde_json::Value,
    version_before: u64,
    version_after: u64,
) -> Result<(), V2Error> {
    let paper_id = paper_id_for_workspace(tx, workspace_id).await?;
    sqlx::query(
        "UPDATE latex_core.reversible_structural_operations \
         SET state='INVALIDATED' WHERE workspace_id=$1 AND state='UNDONE'",
    )
    .bind(workspace_id.as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    sqlx::query(
        "INSERT INTO latex_core.reversible_structural_operations \
         (id,paper_id,workspace_id,actor_user_id,operation_type,file_id,forward_payload,inverse_payload,workspace_version_before,workspace_version_after) \
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(Uuid::new_v4())
    .bind(paper_id)
    .bind(workspace_id.as_uuid())
    .bind(actor.as_uuid())
    .bind(operation_type)
    .bind(file_id)
    .bind(forward_payload)
    .bind(inverse_payload)
    .bind(i64::try_from(version_before).map_err(|_| V2Error::Integrity {
        message: "workspace version exceeds PostgreSQL BIGINT".to_owned(),
    })?)
    .bind(i64::try_from(version_after).map_err(|_| V2Error::Integrity {
        message: "workspace version exceeds PostgreSQL BIGINT".to_owned(),
    })?)
    .execute(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    tracing::info!(%paper_id, %workspace_id, actor_user_id=%actor, %operation_type, workspace_version_before=version_before, workspace_version_after=version_after, "Writer structural operation recorded");
    Ok(())
}

async fn paper_id_for_workspace(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
) -> Result<Uuid, V2Error> {
    sqlx::query_scalar(
        "SELECT id FROM latex_core.personal_papers WHERE workspace_id=$1 \
         UNION ALL SELECT id FROM latex_core.paper_teams WHERE workspace_id=$1",
    )
    .bind(workspace_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound { entity: "paper" })
}

async fn current_workspace_version(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
) -> Result<u64, V2Error> {
    let version: i64 = sqlx::query_scalar(
        "SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1 FOR UPDATE",
    )
    .bind(workspace_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound {
        entity: "workspace",
    })?;
    u64::try_from(version).map_err(|_| V2Error::Integrity {
        message: "negative workspace version".to_owned(),
    })
}

async fn current_main_path(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
) -> Result<Option<LogicalPath>, V2Error> {
    let payloads: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT payload FROM latex_core.workspace_events WHERE workspace_id=$1 ORDER BY sequence",
    )
    .bind(workspace_id.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    let mut main: Option<LogicalPath> = None;
    for payload in payloads {
        let Some(operations) = payload
            .get("operations")
            .and_then(serde_json::Value::as_array)
        else {
            continue;
        };
        for operation in operations {
            match operation.get("op").and_then(serde_json::Value::as_str) {
                Some("set_main_file") => {
                    main = operation
                        .get("path")
                        .cloned()
                        .map(serde_json::from_value)
                        .transpose()
                        .map_err(|error| V2Error::Integrity {
                            message: error.to_string(),
                        })?;
                }
                Some("rename_file") => {
                    let from: LogicalPath = payload_field(operation, "from")?;
                    if main.as_ref() == Some(&from) {
                        main = Some(payload_field(operation, "to")?);
                    }
                }
                Some("delete_file") => {
                    let path: LogicalPath = payload_field(operation, "path")?;
                    if main.as_ref() == Some(&path) {
                        main = None;
                    }
                }
                _ => {}
            }
        }
    }
    Ok(main)
}

fn payload_field<T: serde::de::DeserializeOwned>(
    payload: &serde_json::Value,
    name: &'static str,
) -> Result<T, V2Error> {
    serde_json::from_value(
        payload
            .get(name)
            .cloned()
            .ok_or_else(|| V2Error::Integrity {
                message: format!("structural payload missing {name}"),
            })?,
    )
    .map_err(|error| V2Error::Integrity {
        message: error.to_string(),
    })
}

async fn lock_any_file(
    tx: &mut Transaction<'_, Postgres>,
    file_id: Uuid,
) -> Result<PaperFile, V2Error> {
    let row = sqlx::query(
        "SELECT file_id,workspace_id,path,revision,tombstoned,created_at::text AS created_at,updated_at::text AS updated_at,tombstoned_at::text AS tombstoned_at \
         FROM latex_core.paper_files WHERE file_id=$1 FOR UPDATE",
    )
    .bind(file_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound { entity: "paper file" })?;
    decode_paper_file(row)
}

async fn apply_structural_payload(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
    payload: &serde_json::Value,
) -> Result<Vec<serde_json::Value>, V2Error> {
    let operation = payload
        .get("op")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| V2Error::Integrity {
            message: "structural payload missing op".to_owned(),
        })?;
    match operation {
        "delete_file" => {
            let file_id = payload_field(payload, "file_id")?;
            let path: LogicalPath = payload_field(payload, "path")?;
            let expected_revision: u64 = payload_field(payload, "expected_revision")?;
            let current = lock_any_file(tx, file_id).await?;
            if current.workspace_id != workspace_id
                || current.tombstoned
                || current.path != path
                || current.revision != expected_revision
            {
                return Err(V2Error::Conflict {
                    entity: "file changed since structural operation",
                });
            }
            sqlx::query("UPDATE latex_core.paper_files SET tombstoned=TRUE,tombstoned_at=statement_timestamp(),revision=revision+1,updated_at=statement_timestamp() WHERE file_id=$1")
                .bind(file_id).execute(&mut **tx).await.map_err(V2Error::Database)?;
            Ok(vec![json!({"op":"delete_file","path":path})])
        }
        "create_file" | "restore_file" => {
            let file_id = payload_field(payload, "file_id")?;
            let path: LogicalPath = payload_field(payload, "path")?;
            let blob_hash: BlobHash = payload_field(payload, "blob_hash")?;
            let size_bytes: u64 = payload_field(payload, "size_bytes")?;
            let current = lock_any_file(tx, file_id).await?;
            if current.workspace_id != workspace_id || !current.tombstoned || current.path != path {
                return Err(V2Error::Conflict {
                    entity: "file cannot be restored safely",
                });
            }
            let occupied: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.paper_files WHERE workspace_id=$1 AND path=$2 AND NOT tombstoned)")
                .bind(workspace_id.as_uuid()).bind(path.as_str()).fetch_one(&mut **tx).await.map_err(V2Error::Database)?;
            if occupied {
                return Err(V2Error::Conflict {
                    entity: "restore path is occupied",
                });
            }
            sqlx::query("UPDATE latex_core.paper_files SET tombstoned=FALSE,tombstoned_at=NULL,revision=revision+1,updated_at=statement_timestamp() WHERE file_id=$1")
                .bind(file_id).execute(&mut **tx).await.map_err(V2Error::Database)?;
            let mut operations = vec![
                json!({"op":"put_file","path":path,"blob_hash":blob_hash,"size_bytes":size_bytes}),
            ];
            if payload
                .get("was_main")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                operations.push(json!({"op":"set_main_file","path":path}));
            }
            Ok(operations)
        }
        "rename_file" => {
            let file_id = payload_field(payload, "file_id")?;
            let from: LogicalPath = payload_field(payload, "from")?;
            let to: LogicalPath = payload_field(payload, "to")?;
            let expected_revision: u64 = payload_field(payload, "expected_revision")?;
            let current = lock_any_file(tx, file_id).await?;
            if current.workspace_id != workspace_id
                || current.tombstoned
                || current.path != from
                || current.revision != expected_revision
            {
                return Err(V2Error::Conflict {
                    entity: "file changed since structural operation",
                });
            }
            sqlx::query("UPDATE latex_core.paper_files SET path=$2,revision=revision+1,updated_at=statement_timestamp() WHERE file_id=$1")
                .bind(file_id).bind(to.as_str()).execute(&mut **tx).await.map_err(|error| map_file_conflict(error, workspace_id, &to))?;
            Ok(vec![json!({"op":"rename_file","from":from,"to":to})])
        }
        "set_main" => {
            let from: LogicalPath = payload_field(payload, "from")?;
            let to: LogicalPath = payload_field(payload, "to")?;
            if current_main_path(tx, workspace_id).await?.as_ref() != Some(&from) {
                return Err(V2Error::Conflict {
                    entity: "main file changed since structural operation",
                });
            }
            let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.paper_files WHERE workspace_id=$1 AND path=$2 AND NOT tombstoned)")
                .bind(workspace_id.as_uuid()).bind(to.as_str()).fetch_one(&mut **tx).await.map_err(V2Error::Database)?;
            if !exists {
                return Err(V2Error::Conflict {
                    entity: "prior main file is unavailable",
                });
            }
            Ok(vec![json!({"op":"set_main_file","path":to})])
        }
        _ => Err(V2Error::Integrity {
            message: "unknown structural payload operation".to_owned(),
        }),
    }
}

async fn lock_user(tx: &mut Transaction<'_, Postgres>, user_id: UserId) -> Result<(), V2Error> {
    let exists =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM latex_core.users WHERE id=$1 FOR UPDATE")
            .bind(user_id.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(V2Error::Database)?;
    exists
        .map(|_| ())
        .ok_or(V2Error::NotFound { entity: "user" })
}

async fn lock_users(
    tx: &mut Transaction<'_, Postgres>,
    first: UserId,
    second: UserId,
) -> Result<(), V2Error> {
    let rows = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM latex_core.users WHERE id IN ($1,$2) ORDER BY id FOR UPDATE",
    )
    .bind(first.as_uuid())
    .bind(second.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    let expected = if first == second { 1 } else { 2 };
    if rows.len() == expected {
        Ok(())
    } else {
        Err(V2Error::NotFound { entity: "user" })
    }
}

async fn lock_workspace(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
) -> Result<Uuid, V2Error> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT owner_user_id FROM latex_core.workspaces WHERE id=$1 FOR UPDATE",
    )
    .bind(workspace_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .ok_or(V2Error::NotFound {
        entity: "workspace",
    })
}

async fn lock_paper_team(
    tx: &mut Transaction<'_, Postgres>,
    paper_team_id: Uuid,
) -> Result<(), V2Error> {
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM latex_core.paper_teams WHERE id=$1 FOR UPDATE")
        .bind(paper_team_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(V2Error::Database)?
        .map(|_| ())
        .ok_or(V2Error::NotFound {
            entity: "Paper Team",
        })
}

async fn current_role(
    tx: &mut Transaction<'_, Postgres>,
    user_id: UserId,
) -> Result<Option<GlobalRole>, V2Error> {
    sqlx::query_scalar::<_, String>(
        "SELECT role FROM latex_core.global_user_roles WHERE user_id=$1",
    )
    .bind(user_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .map(|value| GlobalRole::from_str(&value))
    .transpose()
}

async fn require_role(
    tx: &mut Transaction<'_, Postgres>,
    user_id: UserId,
    allowed: &[GlobalRole],
    required: &'static str,
) -> Result<GlobalRole, V2Error> {
    let role = current_role(tx, user_id)
        .await?
        .ok_or(V2Error::RoleMissing { user_id })?;
    if allowed.contains(&role) {
        Ok(role)
    } else {
        Err(V2Error::RoleForbidden {
            user_id,
            required,
            actual: role,
        })
    }
}

async fn validate_role_change(
    tx: &mut Transaction<'_, Postgres>,
    user_id: UserId,
    role: GlobalRole,
) -> Result<(), V2Error> {
    if role != GlobalRole::Writer && owns_personal_paper(tx, user_id).await? {
        return Err(V2Error::PersonalPaperOwnershipConflict { user_id });
    }
    if role == GlobalRole::Admin && has_team_membership(tx, user_id).await? {
        return Err(V2Error::TeamMembershipConflict { user_id });
    }
    if role != GlobalRole::Writer && is_team_leader(tx, user_id).await? {
        return Err(V2Error::TeamMembershipConflict { user_id });
    }
    Ok(())
}

async fn owns_personal_paper(
    tx: &mut Transaction<'_, Postgres>,
    user_id: UserId,
) -> Result<bool, V2Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM latex_core.personal_papers WHERE owner_user_id=$1)",
    )
    .bind(user_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(V2Error::Database)
}

async fn has_team_membership(
    tx: &mut Transaction<'_, Postgres>,
    user_id: UserId,
) -> Result<bool, V2Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM latex_core.paper_team_members WHERE user_id=$1)",
    )
    .bind(user_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(V2Error::Database)
}

async fn is_team_leader(
    tx: &mut Transaction<'_, Postgres>,
    user_id: UserId,
) -> Result<bool, V2Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM latex_core.paper_team_members \
         WHERE user_id=$1 AND is_leader)",
    )
    .bind(user_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(V2Error::Database)
}

async fn active_team_has_one_writer_leader(
    tx: &mut Transaction<'_, Postgres>,
    paper_team_id: Uuid,
) -> Result<bool, V2Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM latex_core.paper_team_members m \
         JOIN latex_core.global_user_roles r ON r.user_id=m.user_id \
         WHERE m.paper_team_id=$1 AND m.is_leader AND r.role='writer'",
    )
    .bind(paper_team_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    Ok(count == 1)
}

async fn revoke_user_sessions(
    tx: &mut Transaction<'_, Postgres>,
    user_id: UserId,
) -> Result<(), V2Error> {
    sqlx::query("DELETE FROM latex_core.sessions WHERE user_id=$1")
        .bind(user_id.as_uuid())
        .execute(&mut **tx)
        .await
        .map_err(V2Error::Database)?;
    Ok(())
}

async fn workspace_is_v2_paper(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
) -> Result<bool, V2Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM latex_core.personal_papers WHERE workspace_id=$1) + \
                (SELECT count(*) FROM latex_core.paper_teams WHERE workspace_id=$1)",
    )
    .bind(workspace_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    Ok(count != 0)
}

async fn require_v2_paper_workspace(
    tx: &mut Transaction<'_, Postgres>,
    workspace_id: WorkspaceId,
) -> Result<(), V2Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM latex_core.personal_papers WHERE workspace_id=$1) + \
                (SELECT count(*) FROM latex_core.paper_teams WHERE workspace_id=$1)",
    )
    .bind(workspace_id.as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(V2Error::Database)?;
    if count == 1 {
        Ok(())
    } else {
        Err(V2Error::WorkspaceConflict { workspace_id })
    }
}

async fn file_workspace(
    tx: &mut Transaction<'_, Postgres>,
    file_id: Uuid,
) -> Result<WorkspaceId, V2Error> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT workspace_id FROM latex_core.paper_files WHERE file_id=$1",
    )
    .bind(file_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(V2Error::Database)?
    .map(WorkspaceId::from_uuid)
    .ok_or(V2Error::NotFound {
        entity: "paper file",
    })
}

fn validate_name(name: &str) -> Result<(), V2Error> {
    if (1..=200).contains(&name.chars().count()) {
        Ok(())
    } else {
        Err(V2Error::InvalidName)
    }
}

fn decode_role_assignment(row: PgRow) -> Result<GlobalRoleAssignment, V2Error> {
    let role: String = row.try_get("role").map_err(V2Error::Database)?;
    Ok(GlobalRoleAssignment {
        user_id: UserId::from_uuid(row.try_get("user_id").map_err(V2Error::Database)?),
        role: GlobalRole::from_str(&role)?,
        created_at: row.try_get("created_at").map_err(V2Error::Database)?,
        updated_at: row.try_get("updated_at").map_err(V2Error::Database)?,
    })
}

fn decode_v2_user(row: PgRow) -> Result<V2User, V2Error> {
    let role: Option<String> = row.try_get("role").map_err(V2Error::Database)?;
    let v2_role = role.as_deref().map(GlobalRole::from_str).transpose()?;
    Ok(V2User {
        user_id: UserId::from_uuid(row.try_get("id").map_err(V2Error::Database)?),
        email: row.try_get("email").map_err(V2Error::Database)?,
        enabled: row.try_get("enabled").map_err(V2Error::Database)?,
        legacy_account_type: row.try_get("account_type").map_err(V2Error::Database)?,
        migration_state: if v2_role.is_some() {
            "ASSIGNED".to_owned()
        } else {
            "UNASSIGNED".to_owned()
        },
        v2_role,
        created_at: row.try_get("created_at").map_err(V2Error::Database)?,
        must_change_password: row
            .try_get("must_change_password")
            .map_err(V2Error::Database)?,
        email_delivery_id: row
            .try_get("email_delivery_id")
            .map_err(V2Error::Database)?,
        email_delivery_status: row
            .try_get("email_delivery_status")
            .map_err(V2Error::Database)?,
        email_delivery_expired: row
            .try_get("email_delivery_expired")
            .map_err(V2Error::Database)?,
    })
}

fn decode_writer_paper(row: PgRow) -> Result<WriterPaper, V2Error> {
    let kind: String = row.try_get("kind").map_err(V2Error::Database)?;
    let status: String = row.try_get("status").map_err(V2Error::Database)?;
    Ok(WriterPaper {
        id: row.try_get("id").map_err(V2Error::Database)?,
        workspace_id: WorkspaceId::from_uuid(
            row.try_get("workspace_id").map_err(V2Error::Database)?,
        ),
        name: row.try_get("name").map_err(V2Error::Database)?,
        kind: match kind.as_str() {
            "personal" => PaperKind::Personal,
            "team" => PaperKind::Team,
            _ => {
                return Err(V2Error::Integrity {
                    message: format!("invalid paper kind: {kind}"),
                });
            }
        },
        status: PaperStatus::from_str(&status)?,
        is_team_leader: row.try_get("is_team_leader").map_err(V2Error::Database)?,
        updated_at: row.try_get("updated_at").map_err(V2Error::Database)?,
    })
}

fn decode_team_member_view(row: PgRow) -> Result<PaperTeamMemberView, V2Error> {
    let role: String = row.try_get("role").map_err(V2Error::Database)?;
    Ok(PaperTeamMemberView {
        user_id: UserId::from_uuid(row.try_get("user_id").map_err(V2Error::Database)?),
        email: row.try_get("email").map_err(V2Error::Database)?,
        role: GlobalRole::from_str(&role)?,
        is_leader: row.try_get("is_leader").map_err(V2Error::Database)?,
        writer_order: row.try_get("writer_order").map_err(V2Error::Database)?,
        created_at: row.try_get("created_at").map_err(V2Error::Database)?,
    })
}

fn decode_personal_paper(row: PgRow) -> Result<PersonalPaper, V2Error> {
    let status: String = row.try_get("status").map_err(V2Error::Database)?;
    Ok(PersonalPaper {
        id: row.try_get("id").map_err(V2Error::Database)?,
        owner_user_id: UserId::from_uuid(row.try_get("owner_user_id").map_err(V2Error::Database)?),
        workspace_id: WorkspaceId::from_uuid(
            row.try_get("workspace_id").map_err(V2Error::Database)?,
        ),
        name: row.try_get("name").map_err(V2Error::Database)?,
        status: PaperStatus::from_str(&status)?,
        created_at: row.try_get("created_at").map_err(V2Error::Database)?,
        updated_at: row.try_get("updated_at").map_err(V2Error::Database)?,
    })
}

pub(crate) fn decode_paper_team(row: PgRow) -> Result<PaperTeam, V2Error> {
    let status: String = row.try_get("status").map_err(V2Error::Database)?;
    Ok(PaperTeam {
        id: row.try_get("id").map_err(V2Error::Database)?,
        workspace_id: WorkspaceId::from_uuid(
            row.try_get("workspace_id").map_err(V2Error::Database)?,
        ),
        name: row.try_get("name").map_err(V2Error::Database)?,
        status: PaperStatus::from_str(&status)?,
        created_by_user_id: UserId::from_uuid(
            row.try_get("created_by_user_id")
                .map_err(V2Error::Database)?,
        ),
        created_at: row.try_get("created_at").map_err(V2Error::Database)?,
        updated_at: row.try_get("updated_at").map_err(V2Error::Database)?,
    })
}

fn decode_paper_team_member(row: PgRow) -> Result<PaperTeamMember, V2Error> {
    Ok(PaperTeamMember {
        paper_team_id: row.try_get("paper_team_id").map_err(V2Error::Database)?,
        user_id: UserId::from_uuid(row.try_get("user_id").map_err(V2Error::Database)?),
        assigned_by_user_id: UserId::from_uuid(
            row.try_get("assigned_by_user_id")
                .map_err(V2Error::Database)?,
        ),
        is_leader: row.try_get("is_leader").map_err(V2Error::Database)?,
        writer_order: row.try_get("writer_order").map_err(V2Error::Database)?,
        created_at: row.try_get("created_at").map_err(V2Error::Database)?,
    })
}

pub(crate) fn decode_paper_file(row: PgRow) -> Result<PaperFile, V2Error> {
    let path: String = row.try_get("path").map_err(V2Error::Database)?;
    let revision: i64 = row.try_get("revision").map_err(V2Error::Database)?;
    Ok(PaperFile {
        file_id: row.try_get("file_id").map_err(V2Error::Database)?,
        workspace_id: WorkspaceId::from_uuid(
            row.try_get("workspace_id").map_err(V2Error::Database)?,
        ),
        path: LogicalPath::parse(&path).map_err(|error| V2Error::InvalidPath {
            message: error.to_string(),
        })?,
        revision: u64::try_from(revision).map_err(|_| V2Error::Integrity {
            message: "paper file revision is negative".to_owned(),
        })?,
        tombstoned: row.try_get("tombstoned").map_err(V2Error::Database)?,
        created_at: row.try_get("created_at").map_err(V2Error::Database)?,
        updated_at: row.try_get("updated_at").map_err(V2Error::Database)?,
        tombstoned_at: row.try_get("tombstoned_at").map_err(V2Error::Database)?,
    })
}

fn map_conflict(error: sqlx::Error, entity: &'static str) -> V2Error {
    if is_unique_violation(&error) {
        V2Error::Conflict { entity }
    } else {
        V2Error::Database(error)
    }
}

fn map_file_conflict(error: sqlx::Error, workspace_id: WorkspaceId, path: &LogicalPath) -> V2Error {
    if constraint(&error) == Some("paper_files_live_workspace_path_unique_idx") {
        V2Error::DuplicateFilePath {
            workspace_id,
            path: path.as_str().to_owned(),
        }
    } else {
        V2Error::Database(error)
    }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    matches!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code),
        Some(code) if code == "23505"
    )
}

fn constraint(error: &sqlx::Error) -> Option<&str> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::constraint)
}
