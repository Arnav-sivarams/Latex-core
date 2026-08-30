//! Structured persistence boundaries.
#![forbid(unsafe_code)]

mod app;
mod collaboration;
#[allow(
    clippy::doc_markdown,
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    reason = "repository methods share the crate's typed error contract; decoding consumes SQLx rows"
)]
mod compile_queue;
mod config;
mod database;
mod error;
mod permissions;
#[allow(
    clippy::missing_errors_doc,
    clippy::needless_pass_by_value,
    reason = "V2 repository methods share a typed domain error and transaction inputs are values"
)]
mod v2;
mod workspace;

pub use app::{
    AdminUserRecord, AppArtifactRecord, AppError, AppJobRecord, AppProjectRecord, AppRepository,
    AppSessionRecord, AppTemplateFileRecord, AppTemplateRecord, AppUserRecord,
    ResearchGroupMemberRecord, ResearchGroupRecord,
};
pub use collaboration::{
    AccountType, ChangeSetPublishResult, FilePolicy, GroupType, MemberDraftRecord,
    PendingChangeSummary, PrivateWorkingTree, ProjectAccess, ProjectedChangeState,
    ProjectedTeamFile, PublishResult, TeamFileRecord, TeamMemberRecord, TeamProjectRecord,
    TeamRecord,
};
pub use compile_queue::{
    CompileCacheRecordV1, CompileJobRecordV1, CompletionOutcome, EnqueueCompileJobV1,
    InfrastructureOutcome, PersistedArtifactV1, PostgresCompileQueue, QueueError, QueueLimits,
};
pub use config::DatabaseConfig;
pub use database::Database;
pub use error::PersistenceError;
pub use permissions::{GroupRoles, OverrideEffect, Permission, PermissionResolver, ProjectRoles};
pub use v2::{
    GlobalRole, GlobalRoleAssignment, PaperFile, PaperStatus, PaperTeam, PaperTeamMember,
    PersonalPaper, V2Error, V2Repository,
};
pub use workspace::{
    PostgresWorkspaceRepository, WorkspaceEventRecord, WorkspaceHeadRecord, WorkspaceSnapshotRecord,
};
