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

use crate::{
    AppError, AppRepository, GroupRoles, OverrideEffect, Permission, PermissionResolver,
    ProjectRoles, ResearchGroupRecord,
};
use core_types::{BlobHash, UserId, WorkspaceId};
use serde_json::json;
use sqlx::{Postgres, Row, Transaction};
use std::{
    collections::{BTreeMap, HashSet},
    str::FromStr,
};
use uuid::Uuid;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum AccountType {
    Student,
    Professor,
    Admin,
}
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GroupType {
    ResearchTeam,
    MentorGroup,
}
impl GroupType {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "research_team" => Ok(Self::ResearchTeam),
            "mentor_group" => Ok(Self::MentorGroup),
            _ => Err(AppError::Integrity {
                message: "invalid group type".into(),
            }),
        }
    }
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ResearchTeam => "research_team",
            Self::MentorGroup => "mentor_group",
        }
    }
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
    Managed,
}
impl FilePolicy {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "editable" => Ok(Self::Editable),
            "read_only" => Ok(Self::ReadOnly),
            "managed" => Ok(Self::Managed),
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
            Self::Managed => "managed",
        }
    }
}

#[derive(Clone, Debug)]
pub struct TeamRecord {
    pub id: Uuid,
    pub name: String,
    pub group_type: GroupType,
    pub created_at: String,
    pub updated_at: String,
}
#[derive(Clone, Debug)]
pub struct TeamMemberRecord {
    pub user_id: UserId,
    pub email: String,
    pub group_manager: bool,
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
pub struct TeamProjectMemberRecord {
    pub user_id: UserId,
    pub email: String,
    pub writer: bool,
    pub mentor: bool,
    pub project_manager: bool,
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
    pub draft_revision: u64,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingChangeSummary {
    pub modified: u64,
    pub added: u64,
    pub renamed: u64,
    pub deleted: u64,
    pub main_changed: bool,
    pub total: u64,
}
#[derive(Clone, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct ProjectedTeamFile {
    pub path: String,
    pub canonical_path: Option<String>,
    pub canonical_revision: Option<u64>,
    pub blob_hash: BlobHash,
    pub size_bytes: u64,
    pub draft_revision: Option<u64>,
    pub added: bool,
    pub modified: bool,
    pub renamed: bool,
    pub pending_delete: bool,
}

/// The primary user-visible state for a projected file.  The boolean fields
/// on `ProjectedTeamFile` deliberately retain the composable internal state
/// (for example, renamed *and* modified); this enum is the collapsed view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProjectedChangeState {
    Canonical,
    Modified,
    Added,
    Renamed,
    PendingDelete,
}

impl ProjectedTeamFile {
    #[must_use]
    pub fn change_state(&self) -> ProjectedChangeState {
        if self.pending_delete {
            ProjectedChangeState::PendingDelete
        } else if self.added {
            ProjectedChangeState::Added
        } else if self.renamed {
            ProjectedChangeState::Renamed
        } else if self.modified {
            ProjectedChangeState::Modified
        } else {
            ProjectedChangeState::Canonical
        }
    }
}
#[derive(Clone, Debug)]
pub struct PrivateWorkingTree {
    pub files: Vec<ProjectedTeamFile>,
    pub pending_main: Option<String>,
    pub summary: PendingChangeSummary,
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
    ResearchGroup {
        group: ResearchGroupRecord,
        account_type: AccountType,
    },
}
impl ProjectAccess {
    #[must_use]
    pub const fn account_type(&self) -> AccountType {
        match self {
            Self::Personal { account_type }
            | Self::Team { account_type, .. }
            | Self::ResearchGroup { account_type, .. } => *account_type,
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChangeSetPublishResult {
    Published {
        canonical_generation: u64,
        workspace_version: u64,
        change_count: u64,
    },
    Conflict {
        conflicts: Vec<ChangeConflict>,
    },
}

/// A deliberately small, client-safe explanation for an atomic publish
/// rejection.  Database details and row versions are never exposed here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeConflict {
    pub path: String,
    pub destination: Option<String>,
    pub reason: ChangeConflictReason,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeConflictReason {
    ChangedSinceEdit,
    DestinationExists,
    MissingSource,
    MainTargetMissing,
    MainLocked,
    PolicyChanged,
    PermissionChanged,
}

impl ChangeConflictReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ChangedSinceEdit => "changed_since_edit",
            Self::DestinationExists => "destination_exists",
            Self::MissingSource => "missing_source",
            Self::MainTargetMissing => "main_target_missing",
            Self::MainLocked => "main_locked",
            Self::PolicyChanged => "policy_changed",
            Self::PermissionChanged => "permission_changed",
        }
    }
}

impl AppRepository {
    /// Returns whether this non-global capability is present on at least one project.
    ///
    /// This is deliberately presentation-only: authorization continues to resolve every
    /// request against the specific project's roles and policy.
    pub async fn has_mentor_project_role(&self, user: UserId) -> Result<bool, AppError> {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM latex_core.team_project_members WHERE user_id=$1 AND mentor=TRUE)",
        )
        .bind(user.as_uuid())
        .fetch_one(self.database.pool())
        .await
        .map_err(AppError::Database)
    }

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
        self.create_group(creator, name, GroupType::ResearchTeam)
            .await
    }
    pub async fn create_group(
        &self,
        creator: UserId,
        name: &str,
        group_type: GroupType,
    ) -> Result<TeamRecord, AppError> {
        let account = self.account_type(creator).await?;
        let overrides = sqlx::query("SELECT permission,effect FROM latex_core.permission_overrides WHERE user_id=$1 AND context_kind='global'")
            .bind(creator.as_uuid()).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        let mut values = Vec::new();
        for row in overrides {
            let permission: String = row.try_get("permission").map_err(AppError::Database)?;
            let effect: String = row.try_get("effect").map_err(AppError::Database)?;
            let effect = match effect.as_str() {
                "allow" => OverrideEffect::Allow,
                "deny" => OverrideEffect::Deny,
                _ => {
                    return Err(AppError::Integrity {
                        message: "invalid permission override effect".into(),
                    });
                }
            };
            values.push((
                Permission::parse(&permission).ok_or_else(|| AppError::Integrity {
                    message: "invalid permission override".into(),
                })?,
                effect,
            ));
        }
        let resolver = PermissionResolver::new(
            account,
            GroupRoles::default(),
            ProjectRoles::default(),
            values,
        );
        let required = if group_type == GroupType::MentorGroup {
            Permission::MentorGroupCreate
        } else {
            Permission::TeamCreate
        };
        if !resolver.allows(required) {
            return Err(AppError::Forbidden);
        }
        if group_type == GroupType::MentorGroup
            && !matches!(account, AccountType::Professor | AccountType::Admin)
        {
            return Err(AppError::Forbidden);
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO latex_core.teams (id,name,created_by,group_type) VALUES ($1,$2,$3,$4)",
        )
        .bind(id)
        .bind(name)
        .bind(creator.as_uuid())
        .bind(group_type.as_str())
        .execute(&mut *tx)
        .await
        .map_err(crate::app::map_conflict)?;
        sqlx::query("INSERT INTO latex_core.team_members (team_id,user_id,group_manager) VALUES ($1,$2,TRUE)").bind(id).bind(creator.as_uuid()).execute(&mut *tx).await.map_err(AppError::Database)?;
        let row = sqlx::query(
            "SELECT id,name,group_type,created_at::text,updated_at::text FROM latex_core.teams WHERE id=$1",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        decode_team(row)
    }

    pub async fn teams_for_user(&self, user: UserId) -> Result<Vec<TeamRecord>, AppError> {
        let rows = sqlx::query("SELECT t.id,t.name,t.group_type,t.created_at::text,t.updated_at::text FROM latex_core.teams t JOIN latex_core.team_members m ON m.team_id=t.id AND m.user_id=$1 ORDER BY t.updated_at DESC,t.id")
            .bind(user.as_uuid()).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_team).collect()
    }

    pub async fn team(&self, actor: UserId, team_id: Uuid) -> Result<TeamRecord, AppError> {
        self.assert_team_visible(actor, team_id).await?;
        let row = sqlx::query(
            "SELECT id,name,group_type,created_at::text,updated_at::text FROM latex_core.teams WHERE id=$1",
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
        let rows = sqlx::query("SELECT m.user_id,c.email,m.group_manager FROM latex_core.team_members m JOIN latex_core.user_credentials c ON c.user_id=m.user_id WHERE m.team_id=$1 ORDER BY c.email")
            .bind(team_id).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_member).collect()
    }

    pub async fn set_group_member(
        &self,
        actor: UserId,
        team_id: Uuid,
        target: UserId,
        group_manager: bool,
    ) -> Result<(), AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        assert_manager(&mut tx, actor, team_id).await?;
        ensure_group_manager_invariant(&mut tx, team_id, target, group_manager).await?;
        sqlx::query("INSERT INTO latex_core.team_members (team_id,user_id,group_manager) VALUES ($1,$2,$3) ON CONFLICT (team_id,user_id) DO UPDATE SET group_manager=EXCLUDED.group_manager")
            .bind(team_id).bind(target.as_uuid()).bind(group_manager).execute(&mut *tx).await.map_err(AppError::Database)?;
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
        ensure_group_manager_invariant(&mut tx, team_id, target, false).await?;
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

    pub async fn unpublished_change_count_for_member(
        &self,
        actor: UserId,
        team_id: Uuid,
        target: UserId,
    ) -> Result<u64, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        assert_manager(&mut tx, actor, team_id).await?;
        let projects: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM latex_core.team_projects WHERE team_id=$1 ORDER BY id",
        )
        .bind(team_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        let mut total = 0_u64;
        for project_id in projects {
            total = total
                .checked_add(
                    private_working_tree_tx(&mut tx, target, project_id)
                        .await?
                        .summary
                        .total,
                )
                .ok_or_else(|| AppError::Integrity {
                    message: "unpublished change count overflow".into(),
                })?;
        }
        tx.commit().await.map_err(AppError::Database)?;
        Ok(total)
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
        let mentor_group: bool = sqlx::query_scalar(
            "SELECT group_type='mentor_group' FROM latex_core.teams WHERE id=$1",
        )
        .bind(team_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        sqlx::query("INSERT INTO latex_core.team_project_members (team_project_id,user_id,writer,mentor,project_manager) VALUES ($1,$2,TRUE,$3,TRUE)")
            .bind(id).bind(actor.as_uuid()).bind(mentor_group).execute(&mut *tx).await.map_err(AppError::Database)?;
        for file in files {
            sqlx::query("INSERT INTO latex_core.team_project_files (team_project_id,logical_path,blob_hash,size_bytes,file_revision) VALUES ($1,$2,$3,$4,$5)")
                .bind(id).bind(&file.path).bind(file.blob_hash.to_hex()).bind(i64_size(file.size_bytes)?).bind(i64_revision(file.revision)?).execute(&mut *tx).await.map_err(AppError::Database)?;
        }
        let row = sqlx::query("SELECT id,team_id,workspace_id,name,canonical_generation,created_at::text,updated_at::text FROM latex_core.team_projects WHERE id=$1").bind(id).fetch_one(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        decode_team_project(row)
    }

    /// Creates a team project from a template's release-critical state.  The
    /// copied rows are deliberately self-contained: later changes to template
    /// audience or rules cannot alter an existing project.
    pub async fn instantiate_team_project_from_template(
        &self,
        actor: UserId,
        team_id: Uuid,
        workspace: WorkspaceId,
        name: &str,
        template_id: Uuid,
    ) -> Result<TeamProjectRecord, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        assert_manager(&mut tx, actor, team_id).await?;
        assert_template_usable_tx(&mut tx, actor, template_id).await?;
        let template = sqlx::query(
            "SELECT main_file,main_file_locked,strict_structure,policy_default FROM latex_core.templates WHERE id=$1 FOR SHARE",
        )
        .bind(template_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(AppError::Database)?
        .ok_or(AppError::NotFound)?;
        let main: String = template.try_get("main_file").map_err(AppError::Database)?;
        let main_locked: bool = template
            .try_get("main_file_locked")
            .map_err(AppError::Database)?;
        let strict_structure: bool = template
            .try_get("strict_structure")
            .map_err(AppError::Database)?;
        let policy_default: String = template
            .try_get("policy_default")
            .map_err(AppError::Database)?;
        FilePolicy::parse(&policy_default)?;
        let files = sqlx::query(
            "SELECT path,blob_hash,size_bytes FROM latex_core.template_files WHERE template_id=$1 ORDER BY path",
        )
        .bind(template_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        if !files.iter().any(|file| {
            file.try_get::<String, _>("path")
                .is_ok_and(|path| path == main)
        }) {
            return Err(AppError::Integrity {
                message: "template main file is absent from template files".into(),
            });
        }
        let id = Uuid::new_v4();
        sqlx::query("INSERT INTO latex_core.team_projects (id,team_id,workspace_id,name,template_id,main_file_locked,strict_structure,policy_default) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(id).bind(team_id).bind(workspace.as_uuid()).bind(name).bind(template_id).bind(main_locked).bind(strict_structure).bind(&policy_default).execute(&mut *tx).await.map_err(crate::app::map_conflict)?;
        let mentor_group: bool = sqlx::query_scalar(
            "SELECT group_type='mentor_group' FROM latex_core.teams WHERE id=$1",
        )
        .bind(team_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        sqlx::query("INSERT INTO latex_core.team_project_members (team_project_id,user_id,writer,mentor,project_manager) VALUES ($1,$2,TRUE,$3,TRUE)")
            .bind(id).bind(actor.as_uuid()).bind(mentor_group).execute(&mut *tx).await.map_err(AppError::Database)?;
        for file in files {
            let path: String = file.try_get("path").map_err(AppError::Database)?;
            let hash: String = file.try_get("blob_hash").map_err(AppError::Database)?;
            let size: i64 = file.try_get("size_bytes").map_err(AppError::Database)?;
            sqlx::query("INSERT INTO latex_core.team_project_files (team_project_id,logical_path,blob_hash,size_bytes,file_revision) VALUES ($1,$2,$3,$4,1)")
                .bind(id).bind(path).bind(hash).bind(size).execute(&mut *tx).await.map_err(AppError::Database)?;
        }
        let policies = sqlx::query(
            "SELECT path_pattern,access_policy FROM latex_core.template_policy_rules WHERE template_id=$1 ORDER BY path_pattern",
        )
        .bind(template_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        for policy in policies {
            let path: String = policy.try_get("path_pattern").map_err(AppError::Database)?;
            let access_policy: String = policy
                .try_get("access_policy")
                .map_err(AppError::Database)?;
            FilePolicy::parse(&access_policy)?;
            sqlx::query("INSERT INTO latex_core.file_policies (team_project_id,logical_path,access_policy,origin,set_by_user_id) VALUES ($1,$2,$3,'template',$4)")
                .bind(id).bind(path).bind(access_policy).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(AppError::Database)?;
        }
        let row = sqlx::query("SELECT id,team_id,workspace_id,name,canonical_generation,created_at::text,updated_at::text FROM latex_core.team_projects WHERE id=$1")
            .bind(id).fetch_one(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        decode_team_project(row)
    }

    pub async fn project_members(
        &self,
        actor: UserId,
        project_id: Uuid,
    ) -> Result<Vec<TeamProjectMemberRecord>, AppError> {
        let access = self.team_access_by_id(actor, project_id).await?;
        let allowed = matches!(
            access,
            ProjectAccess::Team {
                can_manage: true,
                ..
            }
        ) || access.account_type().is_admin();
        if !allowed {
            return Err(AppError::Forbidden);
        }
        let rows = sqlx::query("SELECT pm.user_id,c.email,pm.writer,pm.mentor,pm.project_manager FROM latex_core.team_project_members pm JOIN latex_core.user_credentials c ON c.user_id=pm.user_id WHERE pm.team_project_id=$1 ORDER BY c.email")
            .bind(project_id).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter()
            .map(|row| {
                Ok(TeamProjectMemberRecord {
                    user_id: UserId::from_uuid(row.try_get("user_id").map_err(AppError::Database)?),
                    email: row.try_get("email").map_err(AppError::Database)?,
                    writer: row.try_get("writer").map_err(AppError::Database)?,
                    mentor: row.try_get("mentor").map_err(AppError::Database)?,
                    project_manager: row.try_get("project_manager").map_err(AppError::Database)?,
                })
            })
            .collect()
    }

    pub async fn set_project_member(
        &self,
        actor: UserId,
        project_id: Uuid,
        target: UserId,
        roles: ProjectRoles,
    ) -> Result<(), AppError> {
        if !roles.writer && !roles.mentor && !roles.project_manager {
            return Err(AppError::Integrity {
                message: "a project member must have at least one project role".into(),
            });
        }
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let access = team_access_tx(&mut tx, actor, project_id).await?;
        let resolver = resolver_for_access_tx(&mut tx, actor, &access).await?;
        if !resolver.may_delegate_role(roles) {
            return Err(AppError::Forbidden);
        }
        // Project-management authority is the delegation boundary. A manager
        // must be able to assign each supported project role, including the
        // mentor role whose review capabilities they need not hold themselves.
        let team_id = match access {
            ProjectAccess::Team { ref project, .. } => project.team_id,
            ProjectAccess::Personal { .. } | ProjectAccess::ResearchGroup { .. } => {
                return Err(AppError::NotFound);
            }
        };
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM latex_core.team_members WHERE team_id=$1 AND user_id=$2)",
        )
        .bind(team_id)
        .bind(target.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        if !member {
            return Err(AppError::Integrity {
                message: "a project member must belong to the team".into(),
            });
        }
        ensure_project_manager_invariant(&mut tx, project_id, target, roles.project_manager)
            .await?;
        sqlx::query("INSERT INTO latex_core.team_project_members (team_project_id,user_id,writer,mentor,project_manager) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (team_project_id,user_id) DO UPDATE SET writer=EXCLUDED.writer,mentor=EXCLUDED.mentor,project_manager=EXCLUDED.project_manager")
            .bind(project_id).bind(target.as_uuid()).bind(roles.writer).bind(roles.mentor).bind(roles.project_manager).execute(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)
    }

    pub async fn remove_project_member(
        &self,
        actor: UserId,
        project_id: Uuid,
        target: UserId,
    ) -> Result<(), AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let access = team_access_tx(&mut tx, actor, project_id).await?;
        let resolver = resolver_for_access_tx(&mut tx, actor, &access).await?;
        if !resolver.allows(Permission::ProjectManage) {
            return Err(AppError::Forbidden);
        }
        ensure_project_manager_invariant(&mut tx, project_id, target, false).await?;
        let result = sqlx::query(
            "DELETE FROM latex_core.team_project_members WHERE team_project_id=$1 AND user_id=$2",
        )
        .bind(project_id)
        .bind(target.as_uuid())
        .execute(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        if result.rows_affected() == 0 {
            return Err(AppError::NotFound);
        }
        tx.commit().await.map_err(AppError::Database)
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
        let group = sqlx::query("SELECT g.id,g.name,g.owner_user_id,g.workspace_id,g.created_at::text,g.updated_at::text FROM latex_core.research_groups g JOIN latex_core.research_group_members m ON m.group_id=g.id AND m.user_id=$2 WHERE g.workspace_id=$1")
            .bind(workspace.as_uuid()).bind(user.as_uuid()).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?;
        if let Some(group) = group {
            return Ok(ProjectAccess::ResearchGroup {
                group: ResearchGroupRecord {
                    id: group.try_get("id").map_err(AppError::Database)?,
                    name: group.try_get("name").map_err(AppError::Database)?,
                    owner_user_id: UserId::from_uuid(
                        group.try_get("owner_user_id").map_err(AppError::Database)?,
                    ),
                    workspace_id: WorkspaceId::from_uuid(
                        group.try_get("workspace_id").map_err(AppError::Database)?,
                    ),
                    created_at: group.try_get("created_at").map_err(AppError::Database)?,
                    updated_at: group.try_get("updated_at").map_err(AppError::Database)?,
                },
                account_type,
            });
        }
        let row = sqlx::query("SELECT tp.id,tp.team_id,tp.workspace_id,tp.name,tp.canonical_generation,tp.created_at::text,tp.updated_at::text,COALESCE(pm.writer,FALSE) AS can_write,COALESCE(pm.mentor,FALSE) AS can_mentor,COALESCE(pm.project_manager,FALSE) AS can_manage FROM latex_core.team_projects tp JOIN latex_core.team_project_members pm ON pm.team_project_id=tp.id AND pm.user_id=$2 WHERE tp.workspace_id=$1")
            .bind(workspace.as_uuid()).bind(user.as_uuid()).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
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

    /// The sole domain entry point for effective permissions in a project
    /// context. HTTP handlers must use this instead of reinterpreting roles.
    pub async fn effective_permissions(
        &self,
        user: UserId,
        workspace: WorkspaceId,
    ) -> Result<PermissionResolver, AppError> {
        let access = self.project_access(user, workspace).await?;
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let resolver = resolver_for_access_tx(&mut tx, user, &access).await?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(resolver)
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
            ProjectAccess::Personal { .. } | ProjectAccess::ResearchGroup { .. } => {
                return Err(AppError::NotFound);
            }
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
            ProjectAccess::Personal { .. } | ProjectAccess::ResearchGroup { .. } => {
                return Err(AppError::NotFound);
            }
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

    /// Returns only the caller's normalized private view; canonical state is
    /// never changed while this projection is built.
    pub async fn private_working_tree(
        &self,
        user: UserId,
        project_id: Uuid,
    ) -> Result<PrivateWorkingTree, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let tree = private_working_tree_tx(&mut tx, user, project_id).await?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(tree)
    }

    pub async fn draft_for_user(
        &self,
        user: UserId,
        project_id: Uuid,
        path: &str,
    ) -> Result<Option<MemberDraftRecord>, AppError> {
        let row = sqlx::query("SELECT logical_path,base_file_revision,draft_revision,draft_blob_hash,draft_size_bytes FROM latex_core.member_drafts WHERE team_project_id=$1 AND user_id=$2 AND logical_path=$3")
            .bind(project_id).bind(user.as_uuid()).bind(path).fetch_optional(self.database.pool()).await.map_err(AppError::Database)?;
        row.map(decode_draft).transpose()
    }

    /// Returns the caller's drafts for paths that are not canonical files yet.
    ///
    /// These entries are intentionally kept separate from canonical file listings:
    /// private drafts must be reachable by their author without becoming visible to
    /// other members or the compiler.
    pub async fn draft_only_paths_for_user(
        &self,
        user: UserId,
        project_id: Uuid,
    ) -> Result<Vec<MemberDraftRecord>, AppError> {
        let access = self.team_access_by_id(user, project_id).await?;
        let may_read_own_drafts = matches!(
            access,
            ProjectAccess::Team {
                account_type: AccountType::Admin,
                ..
            } | ProjectAccess::Team {
                can_write: true,
                ..
            }
        );
        if !may_read_own_drafts {
            return Ok(Vec::new());
        }
        let rows = sqlx::query("SELECT d.logical_path,d.base_file_revision,d.draft_revision,d.draft_blob_hash,d.draft_size_bytes FROM latex_core.member_drafts d LEFT JOIN latex_core.team_project_files f ON f.team_project_id=d.team_project_id AND f.logical_path=d.logical_path WHERE d.team_project_id=$1 AND d.user_id=$2 AND f.logical_path IS NULL ORDER BY d.logical_path")
            .bind(project_id).bind(user.as_uuid()).fetch_all(self.database.pool()).await.map_err(AppError::Database)?;
        rows.into_iter().map(decode_draft).collect()
    }

    /// Finds the caller's private draft only when the path has no canonical file.
    /// This prevents a draft saved before a policy change from bypassing a
    /// managed canonical file read rule.
    pub async fn draft_only_for_user(
        &self,
        user: UserId,
        project_id: Uuid,
        path: &str,
    ) -> Result<Option<MemberDraftRecord>, AppError> {
        Ok(self
            .draft_only_paths_for_user(user, project_id)
            .await?
            .into_iter()
            .find(|draft| draft.path == path))
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
        self.save_draft_with_revision(
            user,
            project_id,
            path,
            base_revision,
            blob_hash,
            size_bytes,
            None,
        )
        .await
        .map(|_| ())
    }

    /// Rejects stale saves from another tab while retaining the server's draft.
    pub async fn save_draft_with_revision(
        &self,
        user: UserId,
        project_id: Uuid,
        path: &str,
        base_revision: u64,
        blob_hash: BlobHash,
        size_bytes: u64,
        expected_draft_revision: Option<u64>,
    ) -> Result<u64, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let access = team_access_tx(&mut tx, user, project_id).await?;
        assert_mutable(user, access.clone(), path, &mut tx, project_id).await?;
        let projected = private_working_tree_tx(&mut tx, user, project_id).await?;
        let exists = projected.files.iter().any(|file| {
            !file.pending_delete
                && (file.path == path || file.canonical_path.as_deref() == Some(path))
        });
        if !exists
            && !resolver_for_access_tx(&mut tx, user, &access)
                .await?
                .allows(Permission::FileCreate)
        {
            return Err(AppError::Forbidden);
        }
        if projected.files.iter().any(|file| {
            file.pending_delete
                && (file.path == path || file.canonical_path.as_deref() == Some(path))
        }) || projected.files.iter().any(|file| {
            !file.pending_delete
                && file.canonical_path.as_deref() == Some(path)
                && file.path != path
        }) {
            return Err(AppError::Conflict);
        }
        let revision: Option<i64> = sqlx::query_scalar("SELECT draft_revision FROM latex_core.member_drafts WHERE team_project_id=$1 AND user_id=$2 AND logical_path=$3 FOR UPDATE")
            .bind(project_id).bind(user.as_uuid()).bind(path).fetch_optional(&mut *tx).await.map_err(AppError::Database)?;
        let current = revision.map(db_revision).transpose()?.unwrap_or(0);
        if expected_draft_revision.is_some_and(|expected| expected != current) {
            return Err(AppError::DraftConflict);
        }
        let next = current.checked_add(1).ok_or_else(|| AppError::Integrity {
            message: "draft revision overflow".into(),
        })?;
        sqlx::query("INSERT INTO latex_core.member_drafts (team_project_id,user_id,logical_path,base_file_revision,draft_blob_hash,draft_size_bytes,draft_revision) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (team_project_id,user_id,logical_path) DO UPDATE SET base_file_revision=EXCLUDED.base_file_revision,draft_blob_hash=EXCLUDED.draft_blob_hash,draft_size_bytes=EXCLUDED.draft_size_bytes,draft_revision=EXCLUDED.draft_revision,updated_at=statement_timestamp()")
            .bind(project_id).bind(user.as_uuid()).bind(path).bind(i64_revision(base_revision)?).bind(blob_hash.to_hex()).bind(i64_size(size_bytes)?).bind(i64_revision(next)?).execute(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(next)
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
        assert_mutable(user, access, path, &mut tx, project_id).await?;
        let project = sqlx::query("SELECT id,team_id,workspace_id,name,canonical_generation,created_at::text,updated_at::text FROM latex_core.team_projects WHERE id=$1 FOR UPDATE").bind(project_id).fetch_optional(&mut *tx).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
        let project = decode_team_project(project)?;
        let draft = sqlx::query("SELECT logical_path,base_file_revision,draft_revision,draft_blob_hash,draft_size_bytes FROM latex_core.member_drafts WHERE team_project_id=$1 AND user_id=$2 AND logical_path=$3 FOR UPDATE").bind(project_id).bind(user.as_uuid()).bind(path).fetch_optional(&mut *tx).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
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

    /// Publishes all of one member's staged file writes as one canonical event.
    /// Validation precedes every mutation so a conflict leaves the complete
    /// private change set untouched.
    #[allow(
        clippy::too_many_lines,
        reason = "atomic validation and mutation are deliberately co-located to make the transaction boundary auditable"
    )]
    pub async fn publish_change_set(
        &self,
        user: UserId,
        project_id: Uuid,
    ) -> Result<ChangeSetPublishResult, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let access = team_access_tx(&mut tx, user, project_id).await?;
        // This project lock serializes publication and makes destination
        // absence checks stable while the normalized private tree is built.
        let project = locked_project(&mut tx, project_id).await?;
        let tree = private_working_tree_tx(&mut tx, user, project_id).await?;
        if tree.summary.total == 0 {
            return Err(AppError::NotFound);
        }
        let sources: Vec<String> = tree
            .files
            .iter()
            .filter_map(|file| file.canonical_path.clone())
            .collect();
        let rows = sqlx::query("SELECT logical_path,blob_hash,size_bytes,file_revision FROM latex_core.team_project_files WHERE team_project_id=$1 AND logical_path = ANY($2) ORDER BY logical_path FOR UPDATE")
            .bind(project_id).bind(&sources).fetch_all(&mut *tx).await.map_err(AppError::Database)?;
        let mut canonical = BTreeMap::new();
        for row in rows {
            let path: String = row.try_get("logical_path").map_err(AppError::Database)?;
            canonical.insert(
                path,
                db_revision(row.try_get("file_revision").map_err(AppError::Database)?)?,
            );
        }
        let resolver = resolver_for_access_tx(&mut tx, user, &access).await?;
        let mut conflicts = Vec::new();
        for file in &tree.files {
            let source = file.canonical_path.as_deref();
            let source_revision = source.and_then(|path| canonical.get(path).copied());
            if let (Some(path), Some(expected)) = (source, file.canonical_revision) {
                match source_revision {
                    None => conflicts.push(change_conflict(
                        path,
                        Some(&file.path),
                        ChangeConflictReason::MissingSource,
                    )),
                    Some(actual) if actual != expected => conflicts.push(change_conflict(
                        path,
                        Some(&file.path),
                        ChangeConflictReason::ChangedSinceEdit,
                    )),
                    Some(_) => {}
                }
            }
            if file.renamed
                && !file.pending_delete
                && source != Some(file.path.as_str())
                && canonical.contains_key(&file.path)
            {
                conflicts.push(change_conflict(
                    source.unwrap_or(&file.path),
                    Some(&file.path),
                    ChangeConflictReason::DestinationExists,
                ));
            }
            if file.added && !file.pending_delete && canonical.contains_key(&file.path) {
                conflicts.push(change_conflict(
                    &file.path,
                    None,
                    ChangeConflictReason::DestinationExists,
                ));
            }
            let required: &[Permission] = if file.pending_delete {
                &[Permission::FileDelete]
            } else if file.added {
                // New source content is both a creation and a content write.
                &[Permission::FileCreate, Permission::FileWrite]
            } else if file.renamed && file.modified {
                &[Permission::FileRename, Permission::FileWrite]
            } else if file.renamed {
                &[Permission::FileRename]
            } else if file.modified {
                &[Permission::FileWrite]
            } else {
                continue;
            };
            for action in required {
                if !resolver.allows(*action) {
                    conflicts.push(change_conflict(
                        source.unwrap_or(&file.path),
                        Some(&file.path),
                        ChangeConflictReason::PermissionChanged,
                    ));
                }
            }
            let mut policy_paths = Vec::new();
            if let Some(path) = source {
                policy_paths.push(path);
            }
            if (file.added || file.renamed) && !file.pending_delete {
                policy_paths.push(&file.path);
            }
            for path in policy_paths {
                if policy_for_path_tx(&mut tx, project_id, path).await? != FilePolicy::Editable
                    && !resolver.allows(Permission::FileProtectedWrite)
                {
                    conflicts.push(change_conflict(
                        path,
                        Some(&file.path),
                        ChangeConflictReason::PolicyChanged,
                    ));
                }
            }
        }
        let automatic_main = if tree.pending_main.is_none() {
            workspace_main_tx(&mut tx, project.workspace_id)
                .await?
                .and_then(|current| {
                    tree.files
                        .iter()
                        .find(|file| {
                            !file.pending_delete
                                && file.renamed
                                && file.canonical_path.as_deref() == Some(current.as_str())
                        })
                        .map(|file| file.path.clone())
                })
        } else {
            None
        };
        if let Some(main) = tree.pending_main.as_ref().or(automatic_main.as_ref()) {
            let locked: bool = sqlx::query_scalar(
                "SELECT main_file_locked FROM latex_core.team_projects WHERE id=$1",
            )
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(AppError::Database)?;
            if locked {
                conflicts.push(change_conflict(
                    main,
                    None,
                    ChangeConflictReason::MainLocked,
                ));
            }
            if tree.pending_main.is_some() && !resolver.allows(Permission::FileSetMain) {
                conflicts.push(change_conflict(
                    main,
                    None,
                    ChangeConflictReason::PermissionChanged,
                ));
            }
            if !tree
                .files
                .iter()
                .any(|file| !file.pending_delete && file.path == *main)
            {
                conflicts.push(change_conflict(
                    main,
                    None,
                    ChangeConflictReason::MainTargetMissing,
                ));
            }
        }
        if !conflicts.is_empty() {
            conflicts.sort_by(|left, right| {
                (&left.path, &left.destination, left.reason.as_str()).cmp(&(
                    &right.path,
                    &right.destination,
                    right.reason.as_str(),
                ))
            });
            conflicts.dedup();
            return Ok(ChangeSetPublishResult::Conflict { conflicts });
        }
        let mut operations = Vec::new();
        let mut changed = 0_u64;
        // The delta is calculated only from final projection state, so chains
        // collapse to one rename and rename+modify advances revision once.
        for file in tree
            .files
            .iter()
            .filter(|file| file.pending_delete)
            .filter(|file| file.canonical_path.is_some())
        {
            sqlx::query("DELETE FROM latex_core.team_project_files WHERE team_project_id=$1 AND logical_path=$2")
                .bind(project_id).bind(file.canonical_path.as_deref()).execute(&mut *tx).await.map_err(AppError::Database)?;
            operations.push(json!({"op":"delete_file","path":file.canonical_path}));
            changed += 1;
        }
        for file in tree
            .files
            .iter()
            .filter(|file| !file.pending_delete && file.renamed)
        {
            let source = file.canonical_path.as_deref().ok_or(AppError::Integrity {
                message: "renamed file has no canonical origin".into(),
            })?;
            let revision = file
                .canonical_revision
                .ok_or(AppError::Integrity {
                    message: "renamed file has no canonical revision".into(),
                })?
                .checked_add(1)
                .ok_or_else(|| AppError::Integrity {
                    message: "file revision overflow".into(),
                })?;
            sqlx::query("DELETE FROM latex_core.team_project_files WHERE team_project_id=$1 AND logical_path=$2").bind(project_id).bind(source).execute(&mut *tx).await.map_err(AppError::Database)?;
            sqlx::query("INSERT INTO latex_core.team_project_files (team_project_id,logical_path,blob_hash,size_bytes,file_revision) VALUES ($1,$2,$3,$4,$5)")
                .bind(project_id).bind(&file.path).bind(file.blob_hash.to_hex()).bind(i64_size(file.size_bytes)?).bind(i64_revision(revision)?).execute(&mut *tx).await.map_err(AppError::Database)?;
            operations.push(json!({"op":"rename_file","from":source,"to":file.path}));
            if file.modified {
                operations.push(json!({"op":"put_file","path":file.path,"blob_hash":file.blob_hash.to_hex(),"size_bytes":file.size_bytes}));
            }
            changed += 1;
        }
        for file in tree
            .files
            .iter()
            .filter(|file| !file.pending_delete && !file.renamed && (file.modified || file.added))
        {
            let revision = if file.added {
                1
            } else {
                file.canonical_revision
                    .ok_or(AppError::Integrity {
                        message: "modified file has no canonical revision".into(),
                    })?
                    .checked_add(1)
                    .ok_or_else(|| AppError::Integrity {
                        message: "file revision overflow".into(),
                    })?
            };
            sqlx::query("INSERT INTO latex_core.team_project_files (team_project_id,logical_path,blob_hash,size_bytes,file_revision) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (team_project_id,logical_path) DO UPDATE SET blob_hash=EXCLUDED.blob_hash,size_bytes=EXCLUDED.size_bytes,file_revision=EXCLUDED.file_revision")
                .bind(project_id).bind(&file.path).bind(file.blob_hash.to_hex()).bind(i64_size(file.size_bytes)?).bind(i64_revision(revision)?).execute(&mut *tx).await.map_err(AppError::Database)?;
            operations.push(json!({"op":"put_file","path":file.path,"blob_hash":file.blob_hash.to_hex(),"size_bytes":file.size_bytes}));
            changed += 1;
        }
        if let Some(main) = tree.pending_main.or(automatic_main) {
            operations.push(json!({"op":"set_main_file","path":main}));
        }
        let workspace_version =
            append_workspace_operations(&mut tx, project.workspace_id, user, operations).await?;
        let generation = increment_generation(&mut tx, project_id).await?;
        sqlx::query("INSERT INTO latex_core.audit_events (id,actor_user_id,event_type,resource_type,resource_id,metadata) VALUES ($1,$2,'changes.published','team_project',$3,$4)")
            .bind(Uuid::new_v4()).bind(user.as_uuid()).bind(project_id).bind(json!({"change_count": changed, "canonical_generation": generation})).execute(&mut *tx).await.map_err(AppError::Database)?;
        sqlx::query("DELETE FROM latex_core.member_drafts WHERE team_project_id=$1 AND user_id=$2")
            .bind(project_id)
            .bind(user.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(AppError::Database)?;
        sqlx::query("DELETE FROM latex_core.member_change_operations WHERE team_project_id=$1 AND user_id=$2")
            .bind(project_id).bind(user.as_uuid()).execute(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        Ok(ChangeSetPublishResult::Published {
            canonical_generation: generation,
            workspace_version,
            change_count: changed,
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
        let resolver = resolver_for_access_tx(&mut tx, actor, &access).await?;
        if !resolver.allows(Permission::ProjectManage) {
            return Err(AppError::Forbidden);
        }
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.team_project_files WHERE team_project_id=$1 AND logical_path=$2)").bind(project_id).bind(path).fetch_one(&mut *tx).await.map_err(AppError::Database)?;
        if !exists {
            return Err(AppError::NotFound);
        }
        if !access.account_type().is_admin()
            && let Some(template_policy) =
                template_policy_for_path_tx(&mut tx, project_id, path).await?
            && policy_rank(policy) < policy_rank(template_policy)
        {
            return Err(AppError::Forbidden);
        }
        sqlx::query("INSERT INTO latex_core.file_policies (team_project_id,logical_path,access_policy,origin,set_by_user_id) VALUES ($1,$2,$3,'project_manager',$4) ON CONFLICT (team_project_id,logical_path) DO UPDATE SET access_policy=EXCLUDED.access_policy,set_by_user_id=EXCLUDED.set_by_user_id,updated_at=statement_timestamp()")
            .bind(project_id).bind(path).bind(policy.as_str()).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)
    }

    pub async fn delete_team_file(
        &self,
        actor: UserId,
        project_id: Uuid,
        path: &str,
    ) -> Result<u64, AppError> {
        self.stage_structural_operation(actor, project_id, "delete", Some(path), None)
            .await
    }

    pub async fn rename_team_file(
        &self,
        actor: UserId,
        project_id: Uuid,
        from: &str,
        to: &str,
    ) -> Result<u64, AppError> {
        self.stage_structural_operation(actor, project_id, "rename", Some(from), Some(to))
            .await
    }

    pub async fn set_team_main_file(
        &self,
        actor: UserId,
        project_id: Uuid,
        path: &str,
    ) -> Result<u64, AppError> {
        self.stage_structural_operation(actor, project_id, "set_main", None, Some(path))
            .await
    }

    async fn stage_structural_operation(
        &self,
        actor: UserId,
        project_id: Uuid,
        operation_type: &str,
        source: Option<&str>,
        destination: Option<&str>,
    ) -> Result<u64, AppError> {
        let mut tx = self
            .database
            .pool()
            .begin()
            .await
            .map_err(AppError::Database)?;
        let access = team_access_tx(&mut tx, actor, project_id).await?;
        let resolver = resolver_for_access_tx(&mut tx, actor, &access).await?;
        let needed = match operation_type {
            "delete" => Permission::FileDelete,
            "rename" => Permission::FileRename,
            "set_main" => Permission::FileSetMain,
            _ => {
                return Err(AppError::Integrity {
                    message: "invalid structural operation".into(),
                });
            }
        };
        if !resolver.allows(needed) {
            return Err(AppError::Forbidden);
        }
        if operation_type == "set_main" {
            let locked: bool = sqlx::query_scalar(
                "SELECT main_file_locked FROM latex_core.team_projects WHERE id=$1",
            )
            .bind(project_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(AppError::Database)?;
            if locked {
                return Err(AppError::Forbidden);
            }
        } else {
            for path in [source, destination].into_iter().flatten() {
                if policy_for_path_tx(&mut tx, project_id, path).await? != FilePolicy::Editable
                    && !resolver.allows(Permission::FileProtectedWrite)
                {
                    return Err(AppError::Forbidden);
                }
            }
        }
        let project = locked_project(&mut tx, project_id).await?;
        let tree = private_working_tree_tx(&mut tx, actor, project_id).await?;
        if operation_type == "set_main"
            && !tree
                .files
                .iter()
                .any(|file| !file.pending_delete && Some(file.path.as_str()) == destination)
        {
            return Err(AppError::NotFound);
        }
        if operation_type == "delete" && tree.pending_main.as_deref() == source {
            return Err(AppError::Conflict);
        }
        if operation_type == "set_main" {
            sqlx::query("DELETE FROM latex_core.member_change_operations WHERE team_project_id=$1 AND user_id=$2 AND operation_type='set_main'")
                .bind(project_id).bind(actor.as_uuid()).execute(&mut *tx).await.map_err(AppError::Database)?;
        }
        let base: Option<i64> = match source {
            Some(path) => tree
                .files
                .iter()
                .find(|file| !file.pending_delete && file.path == path)
                .ok_or(AppError::NotFound)?
                .canonical_revision
                .map(i64_revision)
                .transpose()?,
            None => None,
        };
        if operation_type == "rename" {
            let collision = tree
                .files
                .iter()
                .any(|file| !file.pending_delete && Some(file.path.as_str()) == destination);
            if collision {
                return Err(AppError::Conflict);
            }
        }
        let sequence: i64 = sqlx::query_scalar("SELECT COALESCE(max(operation_sequence),0)+1 FROM latex_core.member_change_operations WHERE team_project_id=$1 AND user_id=$2")
            .bind(project_id).bind(actor.as_uuid()).fetch_one(&mut *tx).await.map_err(AppError::Database)?;
        sqlx::query("INSERT INTO latex_core.member_change_operations (id,team_project_id,user_id,operation_sequence,operation_type,source_path,destination_path,base_file_revision) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)")
            .bind(Uuid::new_v4()).bind(project_id).bind(actor.as_uuid()).bind(sequence).bind(operation_type).bind(source).bind(destination).bind(base).execute(&mut *tx).await.map_err(AppError::Database)?;
        let version: i64 = sqlx::query_scalar(
            "SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1",
        )
        .bind(project.workspace_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(AppError::Database)?;
        tx.commit().await.map_err(AppError::Database)?;
        db_revision(version)
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
        let member: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM latex_core.team_members WHERE team_id=$1 AND user_id=$2)",
        )
        .bind(team_id)
        .bind(user.as_uuid())
        .fetch_one(self.database.pool())
        .await
        .map_err(AppError::Database)?;
        if member {
            Ok(())
        } else {
            Err(AppError::NotFound)
        }
    }
}

// This is intentionally a single transaction-local projection: splitting its
// ordered operation application makes it too easy for staging and publishing
// to disagree about the same private tree.
#[allow(clippy::too_many_lines)]
async fn private_working_tree_tx(
    tx: &mut Transaction<'_, Postgres>,
    user: UserId,
    project_id: Uuid,
) -> Result<PrivateWorkingTree, AppError> {
    let rows = sqlx::query("SELECT logical_path,blob_hash,size_bytes,file_revision FROM latex_core.team_project_files WHERE team_project_id=$1 ORDER BY logical_path")
        .bind(project_id).fetch_all(&mut **tx).await.map_err(AppError::Database)?;
    let mut files = Vec::new();
    for row in rows {
        let hash = BlobHash::from_str(
            &row.try_get::<String, _>("blob_hash")
                .map_err(AppError::Database)?,
        )
        .map_err(|e| AppError::Integrity {
            message: e.to_string(),
        })?;
        let size: i64 = row.try_get("size_bytes").map_err(AppError::Database)?;
        files.push(ProjectedTeamFile {
            path: row.try_get("logical_path").map_err(AppError::Database)?,
            canonical_path: Some(row.try_get("logical_path").map_err(AppError::Database)?),
            canonical_revision: Some(db_revision(
                row.try_get("file_revision").map_err(AppError::Database)?,
            )?),
            blob_hash: hash,
            size_bytes: u64::try_from(size).map_err(|_| AppError::Integrity {
                message: "negative file size".into(),
            })?,
            draft_revision: None,
            added: false,
            modified: false,
            renamed: false,
            pending_delete: false,
        });
    }
    // Draft-only files have no structural CREATE row. Load them into the
    // projection before applying operations so a writer can create, then
    // rename or delete, within one private change set. Canonical drafts stay
    // as overlays and are merged after structural operations below.
    let drafts = sqlx::query("SELECT logical_path,base_file_revision,draft_revision,draft_blob_hash,draft_size_bytes FROM latex_core.member_drafts WHERE team_project_id=$1 AND user_id=$2 ORDER BY logical_path")
        .bind(project_id).bind(user.as_uuid()).fetch_all(&mut **tx).await.map_err(AppError::Database)?
        .into_iter().map(decode_draft).collect::<Result<Vec<_>, _>>()?;
    let mut consumed_draft_paths = HashSet::new();
    for draft in &drafts {
        if draft.base_revision == 0 && !files.iter().any(|file| file.path == draft.path) {
            consumed_draft_paths.insert(draft.path.clone());
            files.push(ProjectedTeamFile {
                path: draft.path.clone(),
                canonical_path: None,
                canonical_revision: None,
                blob_hash: draft.blob_hash,
                size_bytes: draft.size_bytes,
                draft_revision: Some(draft.draft_revision),
                added: true,
                modified: false,
                renamed: false,
                pending_delete: false,
            });
        }
    }
    let operations = sqlx::query("SELECT operation_type,source_path,destination_path,base_file_revision FROM latex_core.member_change_operations WHERE team_project_id=$1 AND user_id=$2 ORDER BY operation_sequence")
        .bind(project_id).bind(user.as_uuid()).fetch_all(&mut **tx).await.map_err(AppError::Database)?;
    let mut pending_main = None;
    // A structural operation can consume a draft-only source (there is no
    // canonical row or explicit CREATE operation).  Do not re-add that raw
    // draft after the ordered operation stream has moved or removed it.
    for row in operations {
        let kind: String = row.try_get("operation_type").map_err(AppError::Database)?;
        let source: Option<String> = row.try_get("source_path").map_err(AppError::Database)?;
        let destination: Option<String> = row
            .try_get("destination_path")
            .map_err(AppError::Database)?;
        let base: Option<i64> = row
            .try_get("base_file_revision")
            .map_err(AppError::Database)?;
        let base = base.map(db_revision).transpose()?;
        match kind.as_str() {
            "rename" => {
                let source = source.ok_or(AppError::NotFound)?;
                let destination = destination.ok_or(AppError::NotFound)?;
                if !files
                    .iter()
                    .any(|file| !file.pending_delete && file.path == source)
                {
                    let draft = drafts
                        .iter()
                        .find(|draft| draft.path == source && draft.base_revision == 0)
                        .ok_or(AppError::NotFound)?;
                    files.push(ProjectedTeamFile {
                        path: draft.path.clone(),
                        canonical_path: None,
                        canonical_revision: None,
                        blob_hash: draft.blob_hash,
                        size_bytes: draft.size_bytes,
                        draft_revision: Some(draft.draft_revision),
                        added: true,
                        modified: false,
                        renamed: false,
                        pending_delete: false,
                    });
                    consumed_draft_paths.insert(source.clone());
                }
                if files
                    .iter()
                    .any(|file| !file.pending_delete && file.path == destination)
                {
                    return Err(AppError::Conflict);
                }
                let file = files
                    .iter_mut()
                    .find(|file| !file.pending_delete && file.path == source)
                    .ok_or(AppError::NotFound)?;
                if file.canonical_path.is_some() && base.is_some() {
                    file.canonical_revision = base;
                }
                file.path.clone_from(&destination);
                file.renamed = file.canonical_path.is_some();
                if pending_main.as_deref() == Some(source.as_str()) {
                    pending_main = Some(destination);
                }
            }
            "delete" => {
                let source = source.ok_or(AppError::NotFound)?;
                if !files
                    .iter()
                    .any(|file| !file.pending_delete && file.path == source)
                {
                    let draft = drafts
                        .iter()
                        .find(|draft| draft.path == source && draft.base_revision == 0)
                        .ok_or(AppError::NotFound)?;
                    files.push(ProjectedTeamFile {
                        path: draft.path.clone(),
                        canonical_path: None,
                        canonical_revision: None,
                        blob_hash: draft.blob_hash,
                        size_bytes: draft.size_bytes,
                        draft_revision: Some(draft.draft_revision),
                        added: true,
                        modified: false,
                        renamed: false,
                        pending_delete: false,
                    });
                    consumed_draft_paths.insert(source.clone());
                }
                let index = files
                    .iter()
                    .position(|file| !file.pending_delete && file.path == source)
                    .ok_or(AppError::NotFound)?;
                if files[index].canonical_path.is_some() && base.is_some() {
                    files[index].canonical_revision = base;
                }
                if pending_main.as_deref() == Some(source.as_str()) {
                    return Err(AppError::Conflict);
                }
                if files[index].added {
                    files.remove(index);
                } else {
                    files[index].pending_delete = true;
                }
            }
            "set_main" => {
                pending_main = destination;
            }
            _ => {
                return Err(AppError::Integrity {
                    message: "invalid private operation".into(),
                });
            }
        }
    }
    for row in drafts {
        let draft = row;
        if consumed_draft_paths.contains(&draft.path) {
            continue;
        }
        // A draft can predate a private rename.  Its durable path is then the
        // canonical source path, while the current projected path is the
        // rename destination.  Match that retained origin before treating it
        // as a draft-only addition.
        if let Some(file) = files.iter_mut().find(|file| {
            !file.pending_delete
                && (file.path == draft.path
                    || (file.canonical_path.as_deref() == Some(draft.path.as_str())
                        && file.canonical_revision == Some(draft.base_revision)))
        }) {
            file.blob_hash = draft.blob_hash;
            file.size_bytes = draft.size_bytes;
            file.draft_revision = Some(draft.draft_revision);
            if !file.added {
                file.modified = true;
                // A draft is optimistic state.  Keep its captured canonical
                // revision in the projection so publish can detect a later
                // canonical writer even after a private rename.
                file.canonical_revision = Some(draft.base_revision);
            }
        } else if files
            .iter()
            .any(|file| file.pending_delete && file.path == draft.path)
        {
            return Err(AppError::Conflict);
        } else {
            files.push(ProjectedTeamFile {
                path: draft.path,
                canonical_path: None,
                canonical_revision: None,
                blob_hash: draft.blob_hash,
                size_bytes: draft.size_bytes,
                draft_revision: Some(draft.draft_revision),
                added: true,
                modified: false,
                renamed: false,
                pending_delete: false,
            });
        }
    }
    if pending_main.as_ref().is_some_and(|main| {
        !files
            .iter()
            .any(|file| !file.pending_delete && file.path == *main)
    }) {
        return Err(AppError::Conflict);
    }
    let modified = files
        .iter()
        .filter(|file| file.modified && !file.renamed && !file.pending_delete)
        .count() as u64;
    let added = files
        .iter()
        .filter(|file| file.added && !file.pending_delete)
        .count() as u64;
    let renamed = files
        .iter()
        .filter(|file| file.renamed && !file.pending_delete)
        .count() as u64;
    let deleted = files
        .iter()
        .filter(|file| file.pending_delete && file.canonical_path.is_some())
        .count() as u64;
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let main_changed = pending_main.is_some();
    Ok(PrivateWorkingTree {
        files,
        pending_main,
        summary: PendingChangeSummary {
            modified,
            added,
            renamed,
            deleted,
            main_changed,
            total: modified + added + renamed + deleted + u64::from(main_changed),
        },
    })
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
    let row = sqlx::query("SELECT tp.id,tp.team_id,tp.workspace_id,tp.name,tp.canonical_generation,tp.created_at::text,tp.updated_at::text,COALESCE(pm.writer,FALSE) AS can_write,COALESCE(pm.mentor,FALSE) AS can_mentor,COALESCE(pm.project_manager,FALSE) AS can_manage FROM latex_core.team_projects tp JOIN latex_core.team_project_members pm ON pm.team_project_id=tp.id AND pm.user_id=$2 WHERE tp.id=$1").bind(project_id).bind(user.as_uuid()).fetch_optional(&mut **tx).await.map_err(AppError::Database)?.ok_or(AppError::NotFound)?;
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
    let manager: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.team_members WHERE team_id=$1 AND user_id=$2 AND group_manager)").bind(team_id).bind(user.as_uuid()).fetch_one(&mut **tx).await.map_err(AppError::Database)?;
    if admin || manager {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}
async fn assert_template_usable_tx(
    tx: &mut Transaction<'_, Postgres>,
    user: UserId,
    template_id: Uuid,
) -> Result<(), AppError> {
    let account: String =
        sqlx::query_scalar("SELECT account_type FROM latex_core.user_credentials WHERE user_id=$1")
            .bind(user.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(AppError::Database)?
            .ok_or(AppError::NotFound)?;
    let account = AccountType::parse(&account)?;
    let rows = sqlx::query(
        "SELECT permission,effect FROM latex_core.permission_overrides WHERE user_id=$1 AND context_kind='global'",
    )
    .bind(user.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(AppError::Database)?;
    let mut overrides = Vec::new();
    for row in rows {
        let permission: String = row.try_get("permission").map_err(AppError::Database)?;
        let effect: String = row.try_get("effect").map_err(AppError::Database)?;
        let permission = Permission::parse(&permission).ok_or_else(|| AppError::Integrity {
            message: "invalid permission override".into(),
        })?;
        let effect = match effect.as_str() {
            "allow" => OverrideEffect::Allow,
            "deny" => OverrideEffect::Deny,
            _ => {
                return Err(AppError::Integrity {
                    message: "invalid permission override effect".into(),
                });
            }
        };
        overrides.push((permission, effect));
    }
    if !PermissionResolver::new(
        account,
        GroupRoles::default(),
        ProjectRoles::default(),
        overrides,
    )
    .allows(Permission::TemplateUse)
    {
        return Err(AppError::Forbidden);
    }
    let visible: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM latex_core.templates t LEFT JOIN latex_core.template_account_types a ON a.template_id=t.id AND a.account_type=$2 LEFT JOIN latex_core.template_user_grants g ON g.template_id=t.id AND g.user_id=$1 WHERE t.id=$3 AND (a.template_id IS NOT NULL OR g.user_id IS NOT NULL))")
        .bind(user.as_uuid()).bind(account.as_str()).bind(template_id).fetch_one(&mut **tx).await.map_err(AppError::Database)?;
    if visible {
        Ok(())
    } else {
        Err(AppError::NotFound)
    }
}
async fn resolver_for_access_tx(
    tx: &mut Transaction<'_, Postgres>,
    user: UserId,
    access: &ProjectAccess,
) -> Result<PermissionResolver, AppError> {
    let (account, group, project, team_id, project_id) = match access {
        ProjectAccess::Personal { account_type } => (
            *account_type,
            GroupRoles::default(),
            ProjectRoles {
                writer: true,
                mentor: false,
                project_manager: true,
            },
            None,
            None,
        ),
        ProjectAccess::Team {
            project,
            account_type,
            can_write,
            can_mentor,
            can_manage,
        } => {
            let group_manager: bool = sqlx::query_scalar("SELECT COALESCE((SELECT group_manager FROM latex_core.team_members WHERE team_id=$1 AND user_id=$2),FALSE)")
                .bind(project.team_id).bind(user.as_uuid()).fetch_one(&mut **tx).await.map_err(AppError::Database)?;
            (
                *account_type,
                GroupRoles { group_manager },
                ProjectRoles {
                    writer: *can_write,
                    mentor: *can_mentor,
                    project_manager: *can_manage,
                },
                Some(project.team_id),
                Some(project.id),
            )
        }
        ProjectAccess::ResearchGroup { account_type, .. } => (
            *account_type,
            GroupRoles::default(),
            ProjectRoles {
                writer: true,
                mentor: false,
                project_manager: false,
            },
            None,
            None,
        ),
    };
    let rows = sqlx::query("SELECT permission,effect FROM latex_core.permission_overrides WHERE user_id=$1 AND (context_kind='global' OR (context_kind='team' AND context_id=$2) OR (context_kind='project' AND context_id=$3))")
        .bind(user.as_uuid()).bind(team_id).bind(project_id).fetch_all(&mut **tx).await.map_err(AppError::Database)?;
    let mut overrides = Vec::new();
    for row in rows {
        let permission: String = row.try_get("permission").map_err(AppError::Database)?;
        let effect: String = row.try_get("effect").map_err(AppError::Database)?;
        let permission = Permission::parse(&permission).ok_or_else(|| AppError::Integrity {
            message: "invalid permission override".into(),
        })?;
        let effect = match effect.as_str() {
            "allow" => OverrideEffect::Allow,
            "deny" => OverrideEffect::Deny,
            _ => {
                return Err(AppError::Integrity {
                    message: "invalid permission override effect".into(),
                });
            }
        };
        overrides.push((permission, effect));
    }
    Ok(PermissionResolver::new(account, group, project, overrides))
}
async fn ensure_group_manager_invariant(
    tx: &mut Transaction<'_, Postgres>,
    team_id: Uuid,
    target: UserId,
    target_is_manager: bool,
) -> Result<(), AppError> {
    let is_manager: bool = sqlx::query_scalar("SELECT COALESCE((SELECT group_manager FROM latex_core.team_members WHERE team_id=$1 AND user_id=$2),FALSE)")
        .bind(team_id).bind(target.as_uuid()).fetch_one(&mut **tx).await.map_err(AppError::Database)?;
    if !is_manager || target_is_manager {
        return Ok(());
    }
    let managers = sqlx::query("SELECT m.user_id,c.account_type FROM latex_core.team_members m JOIN latex_core.user_credentials c ON c.user_id=m.user_id WHERE m.team_id=$1 AND m.group_manager FOR UPDATE")
        .bind(team_id).fetch_all(&mut **tx).await.map_err(AppError::Database)?;
    if managers.len() <= 1 {
        return Err(AppError::Integrity {
            message: "a group must retain at least one manager".into(),
        });
    }
    let mentor_group: bool =
        sqlx::query_scalar("SELECT group_type='mentor_group' FROM latex_core.teams WHERE id=$1")
            .bind(team_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;
    if mentor_group {
        let target_qualifies = managers.iter().any(|row| {
            row.try_get::<Uuid, _>("user_id")
                .is_ok_and(|user_id| user_id == *target.as_uuid())
                && row
                    .try_get::<String, _>("account_type")
                    .is_ok_and(|account| account == "professor" || account == "admin")
        });
        let qualifying = managers
            .iter()
            .filter(|row| {
                row.try_get::<String, _>("account_type")
                    .is_ok_and(|account| account == "professor" || account == "admin")
            })
            .count();
        if target_qualifies && qualifying <= 1 {
            return Err(AppError::Integrity {
                message: "a mentor group must retain a professor or administrator manager".into(),
            });
        }
    }
    Ok(())
}

async fn ensure_project_manager_invariant(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    target: UserId,
    target_is_manager: bool,
) -> Result<(), AppError> {
    let is_manager: bool = sqlx::query_scalar("SELECT COALESCE((SELECT project_manager FROM latex_core.team_project_members WHERE team_project_id=$1 AND user_id=$2),FALSE)")
        .bind(project_id)
        .bind(target.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(AppError::Database)?;
    if !is_manager || target_is_manager {
        return Ok(());
    }
    let managers: Vec<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM latex_core.team_project_members WHERE team_project_id=$1 AND project_manager FOR UPDATE",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(AppError::Database)?;
    if managers.len() <= 1 {
        return Err(AppError::Integrity {
            message: "a project must retain at least one project manager".into(),
        });
    }
    Ok(())
}
async fn assert_mutable(
    user: UserId,
    access: ProjectAccess,
    path: &str,
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
) -> Result<(), AppError> {
    let resolver = resolver_for_access_tx(tx, user, &access).await?;
    if !matches!(access, ProjectAccess::Team { .. }) {
        return Err(AppError::NotFound);
    }
    if resolver.allows(Permission::FileWrite)
        && (policy_for_path_tx(tx, project_id, path).await? == FilePolicy::Editable
            || resolver.allows(Permission::FileProtectedWrite))
    {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

fn change_conflict(
    path: &str,
    destination: Option<&str>,
    reason: ChangeConflictReason,
) -> ChangeConflict {
    ChangeConflict {
        path: path.to_owned(),
        destination: destination.map(str::to_owned),
        reason,
    }
}

/// Exact policy wins, then the longest terminal `/*` prefix, then the
/// project default.  Patterns are intentionally not regular expressions.
async fn policy_for_path_tx(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    path: &str,
) -> Result<FilePolicy, AppError> {
    let default: String =
        sqlx::query_scalar("SELECT policy_default FROM latex_core.team_projects WHERE id=$1")
            .bind(project_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(AppError::Database)?;
    let rows = sqlx::query(
        "SELECT logical_path,access_policy FROM latex_core.file_policies WHERE team_project_id=$1",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(AppError::Database)?;
    let mut selected: Option<(usize, String)> = None;
    for row in rows {
        let pattern: String = row.try_get("logical_path").map_err(AppError::Database)?;
        let policy: String = row.try_get("access_policy").map_err(AppError::Database)?;
        if pattern == path {
            return FilePolicy::parse(&policy);
        }
        if let Some(prefix) = pattern.strip_suffix("/*") {
            if path
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('/'))
                && selected
                    .as_ref()
                    .is_none_or(|(length, _)| prefix.len() > *length)
            {
                selected = Some((prefix.len(), policy));
            }
        }
    }
    FilePolicy::parse(&selected.map_or(default, |(_, policy)| policy))
}
async fn template_policy_for_path_tx(
    tx: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    path: &str,
) -> Result<Option<FilePolicy>, AppError> {
    let rows = sqlx::query(
        "SELECT logical_path,access_policy FROM latex_core.file_policies WHERE team_project_id=$1 AND origin='template'",
    )
    .bind(project_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(AppError::Database)?;
    let mut selected: Option<(usize, FilePolicy)> = None;
    for row in rows {
        let pattern: String = row.try_get("logical_path").map_err(AppError::Database)?;
        let policy = FilePolicy::parse(
            &row.try_get::<String, _>("access_policy")
                .map_err(AppError::Database)?,
        )?;
        if pattern == path {
            return Ok(Some(policy));
        }
        if let Some(prefix) = pattern.strip_suffix("/*")
            && path
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('/'))
            && selected
                .as_ref()
                .is_none_or(|(length, _)| prefix.len() > *length)
        {
            selected = Some((prefix.len(), policy));
        }
    }
    Ok(selected.map(|(_, policy)| policy))
}
const fn policy_rank(policy: FilePolicy) -> u8 {
    match policy {
        FilePolicy::Editable => 0,
        FilePolicy::ReadOnly => 1,
        FilePolicy::Managed => 2,
    }
}
fn may_read(_policy: FilePolicy, _account: AccountType, _manager: bool) -> bool {
    // Managed describes edit authority, never confidentiality. Files that enter
    // a TeX workspace must not be represented as secret merely by UI hiding.
    true
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
/// Replays only the Main-pointer portion of durable workspace events. Team
/// projects are created from an initialized workspace, so its initial event
/// supplies the pointer; this keeps a canonical rename from leaving Main
/// dangling without introducing a second source of truth.
async fn workspace_main_tx(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
) -> Result<Option<String>, AppError> {
    let events = sqlx::query(
        "SELECT payload FROM latex_core.workspace_events WHERE workspace_id=$1 ORDER BY sequence",
    )
    .bind(workspace.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(AppError::Database)?;
    let mut main = None;
    for event in events {
        let payload: serde_json::Value = event.try_get("payload").map_err(AppError::Database)?;
        for operation in payload
            .get("operations")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            match operation.get("op").and_then(serde_json::Value::as_str) {
                Some("set_main_file") => {
                    main = operation
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                }
                Some("rename_file")
                    if main.as_deref()
                        == operation.get("from").and_then(serde_json::Value::as_str) =>
                {
                    main = operation
                        .get("to")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                }
                Some("delete_file")
                    if main.as_deref()
                        == operation.get("path").and_then(serde_json::Value::as_str) =>
                {
                    main = None;
                }
                _ => {}
            }
        }
    }
    Ok(main)
}
async fn append_workspace_operation(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    actor: UserId,
    operation: serde_json::Value,
) -> Result<u64, AppError> {
    append_workspace_operations(tx, workspace, actor, vec![operation]).await
}
async fn append_workspace_operations(
    tx: &mut Transaction<'_, Postgres>,
    workspace: WorkspaceId,
    actor: UserId,
    operations: Vec<serde_json::Value>,
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
    let payload = json!({"schema_version":1,"operations":operations});
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
        group_type: GroupType::parse(
            &row.try_get::<String, _>("group_type")
                .map_err(AppError::Database)?,
        )?,
        created_at: row.try_get("created_at").map_err(AppError::Database)?,
        updated_at: row.try_get("updated_at").map_err(AppError::Database)?,
    })
}
fn decode_member(row: sqlx::postgres::PgRow) -> Result<TeamMemberRecord, AppError> {
    Ok(TeamMemberRecord {
        user_id: UserId::from_uuid(row.try_get("user_id").map_err(AppError::Database)?),
        email: row.try_get("email").map_err(AppError::Database)?,
        group_manager: row.try_get("group_manager").map_err(AppError::Database)?,
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
        draft_revision: db_revision(row.try_get("draft_revision").map_err(AppError::Database)?)?,
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
