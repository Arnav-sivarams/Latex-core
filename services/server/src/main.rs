//! Same-origin release API and minimal browser client. This process never invokes Docker.
#![forbid(unsafe_code)]
#![allow(
    clippy::ignored_unit_patterns,
    clippy::manual_let_else,
    clippy::result_large_err,
    clippy::unnecessary_semicolon,
    reason = "HTTP handlers use early response returns to keep authorization checks adjacent to each operation"
)]

mod archive;
mod auth;
mod collaboration;
mod review_api;
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Form, Multipart, Path, Query, State, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use blob_store::{BlobStore, FsBlobStore, FsBlobStoreConfig};
use core_types::{
    ArtifactId, CompileKeyMaterialV1, CostClass, IdempotencyKey, JobId, LatexmkProfileId,
    LogicalPath, ShellPolicy, TexEngine, TexEnvironmentId, UserId, WorkspaceId,
    WorkspaceManifestV1, WorkspaceVersion,
};
use latex_parser::{DiagnosticSeverity, ProjectAnalyzer, ProjectSource, SectionLevel};
use persistence::{
    AccountType, AppError, AppRepository, AppSessionRecord, AppTemplateFileRecord,
    ChangeSetPublishResult, Database, DatabaseConfig, EnqueueCompileJobV1, ExactRestoreState,
    FilePolicy, GlobalRole, GroupType, ImportJobPageFilter, ImportLimits, ImportMode,
    InstitutionBatchUpload, InstitutionError, InstitutionOperation, InstitutionPageFilter,
    InstitutionRepository, PaperTeamPageFilter, PostgresCompileQueue, ProjectAccess, ProjectRoles,
    PublishResult, QueueLimits, TeamFileRecord, TeamTemplateResolutionInput, TemplateChangeFile,
    TemplateChangeRequest, TemplateSeedFile, V2BuildRequest, V2Error, V2FilePolicy, V2Repository,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fmt::Write as _,
    net::SocketAddr,
    str::FromStr,
    sync::Arc,
    time::Duration,
};
use tower_http::{
    limit::RequestBodyLimitLayer, set_header::SetResponseHeaderLayer, trace::TraceLayer,
};
use workspace_model::WorkspaceService;

/// The only browser cookie accepted for authentication.  The previous cookie name is
/// intentionally never parsed for authentication, since browsers can retain it under
/// more than one path scope.
const COOKIE: &str = "latex_core_session_v2";
const LEGACY_COOKIE: &str = "latex_core_session";
const LEGACY_COOKIE_PATHS: [&str; 2] = ["/api", "/api/auth"];
const MAX_FILE_BYTES: usize = 1024 * 1024;

#[derive(Clone)]
struct AppState {
    repo: AppRepository,
    v2: V2Repository,
    institution: InstitutionRepository,
    workspaces: WorkspaceService,
    queue: PostgresCompileQueue,
    blobs: Arc<FsBlobStore>,
    collaboration: collaboration::CollaborationHub,
    environment: TexEnvironmentId,
    cookie_secure: bool,
    allow_registration: bool,
    session_seconds: i64,
}
#[derive(Deserialize)]
struct Credentials {
    email: String,
    password: String,
}
#[derive(Deserialize)]
struct PasswordChangeInput {
    new_password: String,
    confirm_password: String,
}
#[derive(Deserialize)]
struct NewProject {
    name: String,
}
#[derive(Deserialize)]
struct NewTeam {
    name: String,
    group_type: Option<String>,
}
#[derive(Deserialize)]
struct NewResearchGroup {
    name: String,
}
#[derive(Deserialize)]
struct RenameResearchGroup {
    name: String,
}
#[derive(Deserialize)]
struct ResearchGroupMemberInput {
    email: String,
}
#[derive(Deserialize)]
struct AdminUserInput {
    email: String,
    account_type: String,
    password: Option<String>,
}
#[derive(Deserialize)]
struct AdminUserPatch {
    account_type: Option<String>,
    enabled: Option<bool>,
}
#[derive(Deserialize)]
struct AdminPasswordInput {
    password: Option<String>,
}
#[derive(Deserialize)]
struct V2AdminUserInput {
    email: String,
    password: Option<String>,
    role: String,
    #[serde(default)]
    generate_temporary_password: bool,
}
#[derive(Deserialize)]
struct V2RoleInput {
    role: String,
}
#[derive(Deserialize)]
struct V2PaperTeamInput {
    name: String,
    template_id: Option<uuid::Uuid>,
    leader_writer_id: String,
    #[serde(default)]
    writer_ids: Vec<String>,
    #[serde(default)]
    mentor_ids: Vec<String>,
}
#[derive(Deserialize)]
struct V2PaperTeamUpdateInput {
    name: String,
    leader_writer_id: String,
    #[serde(default)]
    writer_ids: Vec<String>,
    #[serde(default)]
    mentor_ids: Vec<String>,
}
#[derive(Deserialize)]
struct V2PaperTeamMemberInput {
    user_id: String,
}

#[derive(Deserialize)]
struct ProgrammeTemplateInput {
    template_id: uuid::Uuid,
}

#[derive(Deserialize)]
struct TemplateResolvePreviewInput {
    ordered_writer_user_ids: Vec<String>,
}

#[derive(Deserialize, Default)]
struct ImportListQuery {
    limit: Option<i64>,
    before: Option<uuid::Uuid>,
    page: Option<i64>,
    search: Option<String>,
    status: Option<String>,
    mode: Option<String>,
    file_type: Option<String>,
}

#[derive(Deserialize, Default)]
struct InstitutionDirectoryQuery {
    limit: Option<i64>,
    page: Option<i64>,
    search: Option<String>,
    programme_code: Option<String>,
    department_id: Option<uuid::Uuid>,
    link_status: Option<String>,
    external_type: Option<String>,
}

#[derive(Deserialize)]
struct ManualIdentityLinkInput {
    external_type: String,
    external_id: String,
    user_id: uuid::Uuid,
}

#[derive(Deserialize)]
struct ManualInstitutionOperationInput {
    operation: String,
    payload: serde_json::Value,
}

#[derive(Deserialize, Default)]
struct V2UserSearchQuery {
    q: Option<String>,
    role: Option<String>,
    limit: Option<i64>,
}

#[derive(Deserialize)]
struct BulkLifecycleInput {
    team_ids: Vec<uuid::Uuid>,
    status: String,
    confirmed: bool,
}

#[derive(Deserialize)]
struct TemplateChangeInput {
    new_template_id: uuid::Uuid,
    #[serde(default)]
    preview_token: Option<String>,
    #[serde(default)]
    confirm_main_file_change: bool,
}

#[derive(Deserialize, Default)]
struct PaperTeamPageQuery {
    limit: Option<i64>,
    page: Option<i64>,
    search: Option<String>,
    status: Option<String>,
    programme_code: Option<String>,
    mentor_user_id: Option<uuid::Uuid>,
    leader_user_id: Option<uuid::Uuid>,
    template_id: Option<uuid::Uuid>,
    review_state: Option<String>,
    source: Option<String>,
    unresolved: Option<bool>,
}
#[derive(Deserialize)]
struct V2PaperInput {
    name: String,
}
#[derive(Deserialize)]
struct V2CreateFileInput {
    path: String,
    content: String,
    version: u64,
}
#[derive(Deserialize)]
struct V2SaveFileInput {
    content: String,
    version: u64,
}
#[derive(Deserialize)]
struct V2RenameFileInput {
    path: String,
    version: u64,
}
#[derive(Deserialize)]
struct V2VersionInput {
    version: u64,
}
#[derive(Deserialize)]
struct V2SearchQuery {
    q: String,
    #[serde(default)]
    case_sensitive: bool,
}
#[derive(Deserialize)]
struct V2AssetQuery {
    path: String,
    version: u64,
}
#[derive(Deserialize)]
struct V2CheckpointInput {
    name: String,
}
#[derive(Deserialize)]
struct V2BuildInput {
    #[serde(default = "default_manual_trigger")]
    trigger_type: String,
}
#[derive(Deserialize)]
struct V2CompareQuery {
    from: uuid::Uuid,
    to: uuid::Uuid,
}
#[derive(Deserialize)]
struct RestorationRequestInput {
    target_version_id: uuid::Uuid,
    reason: Option<String>,
}
#[derive(Deserialize)]
struct GovernanceDecisionInput {
    note: Option<String>,
}
#[derive(Deserialize)]
struct ConfirmedRevertInput {
    confirmed: bool,
}
#[derive(Deserialize)]
struct V2StatusInput {
    status: String,
}
#[derive(Deserialize)]
struct TeamMemberInput {
    email: String,
    group_manager: bool,
}
#[derive(Deserialize)]
struct ProjectMemberInput {
    email: String,
    writer: bool,
    mentor: bool,
    project_manager: bool,
}
#[derive(Deserialize)]
struct TeamProjectInput {
    name: String,
}
#[derive(Deserialize)]
struct TeamTemplateProjectInput {
    name: String,
}
#[derive(Deserialize)]
struct FilePolicyInput {
    policy: String,
}
#[derive(Deserialize)]
struct ImportQuery {
    name: String,
}
#[derive(Deserialize)]
struct V2TemplateEditInput {
    name: String,
    description: Option<String>,
    main_file: String,
}
#[derive(Deserialize)]
struct SetMain {
    path: String,
    version: u64,
}
#[derive(Deserialize)]
struct RenameFile {
    path: String,
}
#[derive(Deserialize, Default)]
struct CompileInput {
    engine: Option<TexEngine>,
    synctex: Option<bool>,
}
#[derive(Serialize)]
struct UserWire {
    id: String,
    email: String,
    account_type: String,
    persona: String,
    landing_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    v2_role: Option<String>,
    capabilities: IdentityCapabilitiesWire,
    must_change_password: bool,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum PrincipalKind {
    V2(GlobalRole),
    Legacy(AccountType),
}

#[derive(Clone, Debug)]
struct AuthenticatedPrincipal {
    session: AppSessionRecord,
    kind: PrincipalKind,
}

impl AuthenticatedPrincipal {
    fn resolve(session: AppSessionRecord) -> Result<Self, AppError> {
        let kind = match session.global_role {
            Some(role) => PrincipalKind::V2(role),
            None => PrincipalKind::Legacy(AccountType::parse(&session.account_type)?),
        };
        Ok(Self { session, kind })
    }

    const fn user_id(&self) -> UserId {
        self.session.user_id
    }

    fn email(&self) -> &str {
        &self.session.email
    }
}

#[derive(Serialize)]
struct V2IdentityWire {
    user_id: String,
    email: String,
    role: String,
}
#[derive(Serialize)]
struct IdentityCapabilitiesWire {
    can_open_admin: bool,
    has_mentor_projects: bool,
}
#[derive(Serialize)]
struct ProjectWire {
    id: String,
    name: String,
    version: u64,
    main_file: Option<String>,
    files: Vec<FileWire>,
    collaboration: Option<ProjectCollaborationWire>,
}
#[derive(Serialize)]
struct FileWire {
    path: String,
    size_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    policy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    draft_revision: Option<u64>,
    has_draft: bool,
}
#[derive(Serialize)]
struct ProjectCollaborationWire {
    team_id: String,
    team_project_id: String,
    canonical_generation: u64,
    can_write: bool,
    can_mentor: bool,
    can_manage: bool,
    account_type: String,
}
#[derive(Serialize)]
struct ProjectListWire {
    id: String,
    name: String,
    created_at: String,
    updated_at: String,
}
#[derive(Serialize)]
struct ResearchGroupWire {
    id: String,
    name: String,
    workspace_id: String,
    owner_user_id: String,
    created_at: String,
    updated_at: String,
}
#[derive(Serialize)]
struct ResearchGroupMemberWire {
    user_id: String,
    email: String,
    joined_at: String,
}
#[derive(Serialize)]
struct JobWire {
    id: String,
    project_id: String,
    state: String,
    snapshot_id: String,
    created_at: String,
    finished_at: Option<String>,
    error: Option<serde_json::Value>,
    queue_position: Option<u64>,
    jobs_ahead: Option<u64>,
}
#[derive(Serialize)]
struct ArtifactWire {
    id: String,
    name: String,
    size_bytes: u64,
    content_type: String,
}
#[derive(Serialize)]
struct VersionWire {
    version: u64,
}
#[derive(Serialize)]
struct ErrorWire {
    error: String,
}
#[derive(Serialize)]
struct TemplateWire {
    id: String,
    name: String,
    description: Option<String>,
    main_file: Option<String>,
}
#[derive(Serialize)]
struct TeamWire {
    id: String,
    name: String,
    group_type: String,
    created_at: String,
    updated_at: String,
}
#[derive(Serialize)]
struct TeamMemberWire {
    user_id: String,
    email: String,
    group_manager: bool,
}
#[derive(Serialize)]
struct ProjectMemberWire {
    user_id: String,
    email: String,
    writer: bool,
    mentor: bool,
    project_manager: bool,
}
#[derive(Serialize)]
struct MemberRemovalWire {
    unpublished_change_count: u64,
}
#[derive(Serialize)]
struct TeamProjectWire {
    id: String,
    workspace_id: String,
    name: String,
    canonical_generation: u64,
}
#[derive(Serialize)]
struct PublishWire {
    published: bool,
    canonical_generation: Option<u64>,
    workspace_version: Option<u64>,
    file_revision: u64,
}
#[derive(Serialize)]
struct ChangeSetPublishWire {
    published: bool,
    canonical_generation: Option<u64>,
    workspace_version: Option<u64>,
    change_count: Option<u64>,
    conflicts: Vec<ChangeConflictWire>,
}
#[derive(Serialize)]
struct ChangeConflictWire {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    destination: Option<String>,
    reason: &'static str,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let database_url = required("DATABASE_URL")?;
    let storage = required("BLOB_STORAGE_ROOT")?;
    let environment = TexEnvironmentId::parse(&required("TEX_ENVIRONMENT_ID")?)?;
    let database = Database::connect(DatabaseConfig::development(database_url)?).await?;
    database.migrate().await?;
    let blobs =
        Arc::new(FsBlobStore::open(storage, FsBlobStoreConfig::development_default()).await?);
    let queue = PostgresCompileQueue::new(database.clone(), queue_limits()?);
    let v2 = V2Repository::new(database.clone());
    let workspaces = WorkspaceService::new(
        persistence::PostgresWorkspaceRepository::new(database.clone()),
        blobs.clone(),
    );
    let collaboration =
        collaboration::CollaborationHub::new(v2.clone(), workspaces.clone(), blobs.clone());
    let state = AppState {
        repo: AppRepository::new(database.clone()),
        institution: InstitutionRepository::new(database.clone()),
        v2,
        workspaces,
        queue,
        blobs,
        collaboration,
        environment,
        cookie_secure: bool_env("SESSION_COOKIE_SECURE", false),
        allow_registration: bool_env("ALLOW_REGISTRATION", false),
        session_seconds: int_env("SESSION_TTL_SECONDS", 60 * 60 * 24 * 7)?,
    };
    let app = router(state);
    let address: SocketAddr = env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
        .parse()?;
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "API listening");
    axum::serve(listener, app).await?;
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "routes remain auditable at the application boundary"
)]
fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(ui))
        .route("/login", post(browser_login))
        .route("/logout", post(browser_logout))
        .route(
            "/change-password",
            get(change_password_ui).post(browser_change_password),
        )
        .route("/admin", get(admin_ui))
        .route("/write", get(writer_ui))
        .route("/review", get(mentor_ui))
        .route("/account-setup", get(account_setup_ui))
        .route("/workspace", get(workspace_ui))
        .route("/static/styles.css", get(styles))
        .route("/static/shells.css", get(shells_css))
        .route("/static/admin.js", get(admin_js))
        .route("/static/writer.js", get(writer_js))
        .route("/static/app.js", get(app_js))
        .route("/static/api.js", get(api_js))
        .route("/static/state.js", get(state_js))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/change-password", post(change_password))
        .route("/api/auth/me", get(me))
        .route("/api/v2/me", get(v2_me))
        .route(
            "/api/admin/v2/users",
            get(admin_v2_users).post(admin_v2_create_user),
        )
        .route(
            "/api/admin/v2/users/{user_id}/role",
            axum::routing::patch(admin_v2_change_role),
        )
        .route(
            "/api/admin/v2/users/{user_id}/temporary-password",
            post(admin_v2_reset_temporary_password),
        )
        .route(
            "/api/admin/v2/templates/preview",
            post(admin_v2_template_preview),
        )
        .route(
            "/api/admin/v2/templates/import",
            post(admin_v2_template_import),
        )
        .route(
            "/api/admin/v2/templates/{id}",
            axum::routing::patch(admin_v2_template_edit).delete(admin_v2_template_remove),
        )
        .route(
            "/api/admin/v2/paper-teams",
            get(admin_v2_paper_teams).post(admin_v2_create_paper_team),
        )
        .route(
            "/api/admin/v2/paper-teams/query",
            get(admin_v2_paper_teams_query),
        )
        .route(
            "/api/admin/v2/paper-teams/bulk-lifecycle",
            post(admin_v2_bulk_paper_team_lifecycle),
        )
        .route(
            "/api/admin/v2/institution/imports/validate",
            post(admin_v2_institution_validate),
        )
        .route(
            "/api/admin/v2/institution/import-batches/validate",
            post(admin_v2_institution_batch_validate),
        )
        .route(
            "/api/admin/v2/institution/import-batches",
            get(admin_v2_institution_batches),
        )
        .route(
            "/api/admin/v2/institution/import-batches/{batch_id}",
            get(admin_v2_institution_batch),
        )
        .route(
            "/api/admin/v2/institution/import-batches/{batch_id}/apply",
            post(admin_v2_institution_batch_apply),
        )
        .route(
            "/api/admin/v2/institution/imports/{job_id}/apply",
            post(admin_v2_institution_apply),
        )
        .route(
            "/api/admin/v2/institution/imports/{job_id}/retry-teams",
            post(admin_v2_institution_retry_teams),
        )
        .route(
            "/api/admin/v2/institution/imports",
            get(admin_v2_institution_imports),
        )
        .route(
            "/api/admin/v2/institution/imports/{job_id}",
            get(admin_v2_institution_import),
        )
        .route(
            "/api/admin/v2/institution/imports/{job_id}/errors.csv",
            get(admin_v2_institution_errors),
        )
        .route(
            "/api/admin/v2/institution/students",
            get(admin_v2_institution_students),
        )
        .route(
            "/api/admin/v2/institution/faculty",
            get(admin_v2_institution_faculty),
        )
        .route(
            "/api/admin/v2/institution/programmes",
            get(admin_v2_institution_programmes),
        )
        .route(
            "/api/admin/v2/institution/data/{dataset}",
            get(admin_v2_institution_dataset),
        )
        .route(
            "/api/admin/v2/institution/data/{dataset}/validate",
            post(admin_v2_institution_manual_validate),
        )
        .route(
            "/api/admin/v2/institution/identity-links",
            get(admin_v2_identity_links).put(admin_v2_manual_identity_link),
        )
        .route(
            "/api/admin/v2/institution/identity-links/{external_type}/{external_id}",
            axum::routing::delete(admin_v2_unlink_identity),
        )
        .route(
            "/api/admin/v2/institution/users/search",
            get(admin_v2_institution_user_search),
        )
        .route(
            "/api/admin/v2/institution/template-defaults/programmes",
            get(admin_v2_programme_template_defaults),
        )
        .route(
            "/api/admin/v2/institution/template-defaults/programmes/{programme_code}",
            axum::routing::put(admin_v2_set_programme_template_default)
                .delete(admin_v2_delete_programme_template_default),
        )
        .route(
            "/api/admin/v2/institution/template-defaults/resolve-preview",
            post(admin_v2_template_resolve_preview),
        )
        .route(
            "/api/admin/v2/institution/template-defaults/global-fallback",
            get(admin_v2_global_fallback).put(admin_v2_set_global_fallback),
        )
        .route(
            "/api/admin/v2/paper-teams/{id}",
            get(admin_v2_paper_team).put(admin_v2_update_paper_team),
        )
        .route(
            "/api/admin/v2/paper-teams/{id}/template-change/preview",
            post(admin_v2_template_change_preview),
        )
        .route(
            "/api/admin/v2/paper-teams/{id}/template-change/apply",
            post(admin_v2_template_change_apply),
        )
        .route(
            "/api/admin/v2/paper-teams/{id}/members",
            post(admin_v2_add_paper_team_member),
        )
        .route(
            "/api/admin/v2/paper-teams/{id}/members/{user_id}",
            axum::routing::delete(admin_v2_remove_paper_team_member),
        )
        .route(
            "/api/admin/v2/paper-teams/{id}/leader",
            axum::routing::patch(admin_v2_change_paper_team_leader),
        )
        .route(
            "/api/admin/v2/paper-teams/{id}/status",
            axum::routing::patch(admin_v2_paper_team_status),
        )
        .route(
            "/api/admin/v2/paper-teams/{id}/file-policies",
            get(admin_v2_file_policies),
        )
        .route(
            "/api/admin/v2/paper-teams/{id}/file-policies/{file_id}",
            axum::routing::patch(admin_v2_set_file_policy),
        )
        .route(
            "/api/admin/v2/restoration-requests",
            get(admin_v2_restoration_requests),
        )
        .route(
            "/api/admin/v2/restoration-requests/{request_id}/reject",
            post(gone_restoration_governance),
        )
        .route(
            "/api/admin/v2/restoration-requests/{request_id}/apply",
            post(gone_restoration_governance),
        )
        .route("/api/admin/v2/versions", get(admin_v2_versions))
        .route("/api/admin/v2/reviews", get(admin_v2_reviews))
        .route("/api/v2/writer/papers", get(v2_writer_papers))
        .route(
            "/api/v2/writer/personal-papers",
            post(v2_create_personal_paper),
        )
        .route("/api/v2/papers/{paper_id}", get(v2_paper))
        .route(
            "/api/v2/papers/{paper_id}/files",
            get(v2_paper_files).post(v2_create_file),
        )
        .route(
            "/api/v2/papers/{paper_id}/files/{file_id}",
            get(v2_file).put(v2_save_file).delete(v2_delete_file),
        )
        .route(
            "/api/v2/collab/{paper_id}/files/{file_id}",
            get(v2_collaboration_socket),
        )
        .route(
            "/api/v2/papers/{paper_id}/files/{file_id}/path",
            axum::routing::patch(v2_rename_file),
        )
        .route(
            "/api/v2/papers/{paper_id}/main/{file_id}",
            post(v2_set_main),
        )
        .route(
            "/api/v2/papers/{paper_id}/structural-undo",
            post(v2_structural_undo),
        )
        .route(
            "/api/v2/papers/{paper_id}/structural-redo",
            post(v2_structural_redo),
        )
        .route(
            "/api/v2/papers/{paper_id}/intelligence",
            get(v2_paper_intelligence),
        )
        .route("/api/v2/papers/{paper_id}/search", get(v2_project_search))
        .route("/api/v2/papers/{paper_id}/assets", post(v2_upload_asset))
        .route(
            "/api/v2/papers/{paper_id}/files/{file_id}/raw",
            get(v2_raw_file),
        )
        .route(
            "/api/v2/papers/{paper_id}/versions",
            get(v2_versions).post(v2_create_checkpoint),
        )
        .route(
            "/api/v2/papers/{paper_id}/versions/compare",
            get(v2_compare_versions),
        )
        .route(
            "/api/v2/papers/{paper_id}/versions/{version_id}",
            get(v2_version),
        )
        .route(
            "/api/v2/papers/{paper_id}/versions/{version_id}/restore",
            post(v2_restore_personal_version),
        )
        .route(
            "/api/v2/papers/{paper_id}/versions/{version_id}/revert",
            post(v2_direct_team_revert),
        )
        .route(
            "/api/v2/papers/{paper_id}/restoration-requests",
            get(v2_restoration_requests).post(v2_create_restoration_request),
        )
        .route(
            "/api/v2/restoration-requests/{request_id}/submit",
            post(gone_restoration_governance),
        )
        .route(
            "/api/v2/restoration-requests/{request_id}/endorse",
            post(gone_restoration_governance),
        )
        .route(
            "/api/v2/restoration-requests/{request_id}/reject",
            post(v2_leader_reject_restoration),
        )
        .route(
            "/api/v2/restoration-requests/{request_id}/apply",
            post(v2_leader_apply_restoration),
        )
        .route(
            "/api/v2/restoration-requests",
            get(v2_assigned_restoration_requests),
        )
        .route(
            "/api/v2/papers/{paper_id}/builds",
            get(v2_build_status).post(v2_submit_build),
        )
        .route(
            "/api/v2/papers/{paper_id}/artifacts/{kind}",
            get(v2_current_artifact),
        )
        .route("/api/admin/overview", get(admin_overview))
        .route("/api/admin/users", get(admin_users).post(admin_create_user))
        .route(
            "/api/admin/users/{email}",
            axum::routing::patch(admin_patch_user).delete(admin_delete_user),
        )
        .route(
            "/api/admin/users/{email}/reset-password",
            post(admin_reset_password),
        )
        .route("/api/admin/teams", get(admin_teams))
        .route("/api/admin/research-groups", get(admin_research_groups))
        .route("/api/admin/projects", get(admin_projects))
        .route("/api/admin/templates", get(admin_templates))
        .route("/api/admin/jobs", get(admin_jobs))
        .route("/api/admin/audit", get(admin_audit))
        .route("/api/admin/system", get(admin_system))
        .route("/api/projects", get(projects).post(create_project))
        .route("/api/projects/import", post(import_project))
        .route("/api/projects/{id}", get(project))
        .route("/api/projects/{id}/files", get(files))
        .route(
            "/api/projects/{id}/files/{*path}",
            get(file)
                .put(put_file)
                .patch(rename_file)
                .delete(delete_file),
        )
        .route("/api/projects/{id}/main", post(set_main))
        .route("/api/projects/{id}/compile", post(submit_compile))
        .route(
            "/api/research-groups",
            get(research_groups).post(create_research_group),
        )
        .route(
            "/api/research-groups/{id}",
            get(research_group)
                .patch(rename_research_group)
                .delete(delete_research_group),
        )
        .route(
            "/api/research-groups/{id}/members",
            get(research_group_members).post(add_research_group_member),
        )
        .route(
            "/api/research-groups/{id}/members/{user}",
            axum::routing::delete(remove_research_group_member),
        )
        .route("/api/teams", get(teams).post(create_team))
        .route("/api/teams/{id}", get(team))
        .route(
            "/api/teams/{id}/members",
            get(team_members).post(set_team_member),
        )
        .route(
            "/api/teams/{id}/members/{user}",
            axum::routing::delete(remove_team_member),
        )
        .route(
            "/api/teams/{id}/projects",
            get(team_projects).post(create_team_project),
        )
        .route(
            "/api/teams/{team}/templates/{template}/projects",
            post(create_team_project_from_template),
        )
        .route(
            "/api/team-projects/{id}/drafts/{*path}",
            post(publish_draft),
        )
        .route("/api/team-projects/{id}/publish", post(publish_change_set))
        .route(
            "/api/team-projects/{id}/members",
            get(project_members).post(set_project_member),
        )
        .route(
            "/api/team-projects/{id}/members/{user}",
            axum::routing::delete(remove_project_member),
        )
        .route(
            "/api/team-projects/{id}/policies/{*path}",
            post(set_file_policy),
        )
        .route("/api/templates", get(templates))
        .route(
            "/api/templates/{id}/projects",
            post(create_project_from_template),
        )
        .route("/api/jobs/{id}", get(job))
        .route("/api/jobs/{id}/cancel", post(cancel))
        .route("/api/jobs/{id}/artifacts", get(artifacts))
        .route("/api/jobs/{id}/artifacts/{artifact}", get(artifact))
        .merge(review_api::router())
        .layer(RequestBodyLimitLayer::new(
            archive::MAX_ARCHIVE_BYTES + 64 * 1024,
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::overriding(
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Credentials>,
) -> Response {
    if !state.allow_registration {
        return error(StatusCode::FORBIDDEN, "registration is disabled");
    }
    if let Err(r) = csrf(&headers) {
        return r;
    }
    let email = match auth::normalized_email(&input.email) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid credentials"),
    };
    let hash = match auth::hash_password(&input.password) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "authentication failure"),
    };
    match state.repo.create_account(&email, &hash).await {
        Ok(user) => session_response(&state, &headers, user.user_id, user.email, false).await,
        Err(AppError::Conflict) => error(StatusCode::CONFLICT, "account exists"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<Credentials>,
) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    };
    let Some((user, email, must_change_password)) = (match valid_credentials(&state, &input).await {
        Ok(value) => value,
        Err(response) => return response,
    }) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    session_response(&state, &headers, user, email, must_change_password).await
}

async fn browser_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(input): Form<Credentials>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let Some((user, _email, must_change_password)) = (match valid_credentials(&state, &input).await
    {
        Ok(value) => value,
        Err(response) => return response,
    }) else {
        return (
            StatusCode::UNAUTHORIZED,
            Html(login_html(Some("Invalid email or password."))),
        )
            .into_response();
    };
    let cookies = match create_session_headers(&state, &headers, user).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let location = match principal_kind_for_user(&state, user).await {
        Ok(kind) => landing_path_for(kind, must_change_password),
        Err(response) => return response,
    };
    redirect_with_cookies(location, cookies)
}

async fn valid_credentials(
    state: &AppState,
    input: &Credentials,
) -> Result<Option<(UserId, String, bool)>, Response> {
    let email = match auth::normalized_email(&input.email) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let user = match state.repo.user_by_email(&email).await {
        Ok(value) => value,
        Err(_) => {
            return Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "authentication failure",
            ));
        }
    };
    let Some(user) = user else {
        return Ok(None);
    };
    if !user.enabled {
        return Ok(None);
    }
    let valid = auth::verify_password(&input.password, &user.password_hash)
        .map_err(|()| error(StatusCode::INTERNAL_SERVER_ERROR, "authentication failure"))?;
    Ok(valid.then_some((user.user_id, user.email, user.must_change_password)))
}

async fn change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<PasswordChangeInput>,
) -> Response {
    complete_password_change(&state, &headers, &input, false).await
}

async fn browser_change_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(input): Form<PasswordChangeInput>,
) -> Response {
    complete_password_change(&state, &headers, &input, true).await
}

async fn complete_password_change(
    state: &AppState,
    headers: &HeaderMap,
    input: &PasswordChangeInput,
    browser: bool,
) -> Response {
    if let Err(response) = csrf(headers) {
        return response;
    }
    let principal = match principal_auth_allow_temporary(state, headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !principal.session.must_change_password {
        return error(StatusCode::CONFLICT, "password change is not required");
    }
    if input.new_password != input.confirm_password {
        return password_change_error(browser, "Passwords do not match.");
    }
    let hash = match auth::hash_password(&input.new_password) {
        Ok(value) => value,
        Err(_) => {
            return password_change_error(browser, "Password must be 12–256 characters.");
        }
    };
    if let Err(error_value) = state
        .repo
        .complete_password_change(principal.user_id(), &hash)
        .await
    {
        return match error_value {
            AppError::Conflict => error(StatusCode::CONFLICT, "password already changed"),
            _ => error(StatusCode::INTERNAL_SERVER_ERROR, "password change failed"),
        };
    }
    let cookies = match create_session_headers(state, headers, principal.user_id()).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if browser {
        redirect_with_cookies(landing_path(principal.kind), cookies)
    } else {
        (StatusCode::NO_CONTENT, cookies).into_response()
    }
}

fn password_change_error(browser: bool, message: &'static str) -> Response {
    if browser {
        (
            StatusCode::BAD_REQUEST,
            Html(change_password_html(Some(message))),
        )
            .into_response()
    } else {
        error(StatusCode::BAD_REQUEST, message)
    }
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    };
    let cookies = match revoke_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    (StatusCode::NO_CONTENT, cookies).into_response()
}

async fn browser_logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let cookies = match revoke_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    redirect_with_cookies("/", cookies)
}

async fn revoke_session(state: &AppState, headers: &HeaderMap) -> Result<HeaderMap, Response> {
    for token in named_cookies(headers, COOKIE) {
        if state.repo.delete_session(&digest(&token)).await.is_err() {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"));
        }
    }
    expired_session_cookie_headers(state.cookie_secure)
        .map_err(|()| error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"))
}
async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match principal_auth(&state, &headers).await {
        Ok(principal) => identity_response(&state, principal).await,
        Err(r) => r,
    }
}

async fn v2_me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match principal_auth(&state, &headers).await {
        Ok(principal) => principal,
        Err(response) => return response,
    };
    let PrincipalKind::V2(role) = principal.kind else {
        return error(StatusCode::FORBIDDEN, "V2 principal required");
    };
    Json(V2IdentityWire {
        user_id: principal.user_id().to_string(),
        email: principal.email().to_owned(),
        role: role.as_str().to_owned(),
    })
    .into_response()
}

const INITIAL_TEX: &str =
    "\\documentclass{article}\n\\begin{document}\nStart writing your paper.\n\\end{document}\n";

async fn admin_v2_users(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    match state.v2.list_v2_users().await {
        Ok(users) => Json(users).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn admin_v2_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<V2AdminUserInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    let email = match auth::normalized_email(&input.email) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid email"),
    };
    let role = match input.role.parse::<GlobalRole>() {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid V2 role"),
    };
    if role == GlobalRole::Admin && input.generate_temporary_password {
        return error(
            StatusCode::BAD_REQUEST,
            "temporary-password provisioning is limited to Writer and Mentor",
        );
    }
    let temporary_password = input
        .generate_temporary_password
        .then(auth::temporary_password);
    let password_hash_result = match temporary_password.as_deref() {
        Some(value) => auth::hash_temporary_password(value),
        None => input
            .password
            .as_deref()
            .ok_or(())
            .and_then(auth::hash_password),
    };
    let password_hash = match password_hash_result {
        Ok(value) => value,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "password must be 12-256 characters",
            );
        }
    };
    match state
        .repo
        .create_v2_account(&email, &password_hash, role, temporary_password.is_some())
        .await
    {
        Ok(user) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "user_id": user.user_id,
                "email": user.email,
                "enabled": user.enabled,
                "role": role,
                "must_change_password": user.must_change_password,
                "temporary_password": temporary_password
            })),
        )
            .into_response(),
        Err(AppError::Conflict) => error(StatusCode::CONFLICT, "email already exists"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "account creation failed"),
    }
}

async fn admin_v2_reset_temporary_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let user_id = match parse_user_id(&user_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let temporary_password = auth::temporary_password();
    let password_hash = match auth::hash_temporary_password(&temporary_password) {
        Ok(value) => value,
        Err(_) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "credential generation failed",
            );
        }
    };
    match state
        .repo
        .reset_v2_temporary_password(principal.user_id(), user_id, &password_hash)
        .await
    {
        Ok(email) => Json(serde_json::json!({
            "email":email,
            "temporary_password":temporary_password,
            "must_change_password":true
        }))
        .into_response(),
        Err(AppError::Forbidden) => error(
            StatusCode::FORBIDDEN,
            "temporary passwords can only be generated for Writer or Mentor",
        ),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "account not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "password reset failed"),
    }
}

async fn admin_v2_change_role(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(user_id): Path<String>,
    Json(input): Json<V2RoleInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    let user_id = match parse_user_id(&user_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let role = match input.role.parse::<GlobalRole>() {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid V2 role"),
    };
    match state.v2.set_global_role(user_id, role).await {
        Ok(assignment) => Json(assignment).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn admin_v2_paper_teams(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    match state.v2.list_paper_teams().await {
        Ok(teams) => Json(teams).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn admin_v2_paper_teams_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PaperTeamPageQuery>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    let filter = PaperTeamPageFilter {
        limit: query.limit.unwrap_or(50),
        page: query.page.unwrap_or(1),
        search: query.search,
        status: query.status,
        programme_code: query.programme_code,
        mentor_user_id: query.mentor_user_id,
        leader_user_id: query.leader_user_id,
        template_id: query.template_id,
        review_state: query.review_state,
        source: query.source,
        unresolved: query.unresolved,
    };
    match state.institution.paginated_paper_teams(&filter).await {
        Ok(page) => Json(page).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_bulk_paper_team_lifecycle(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<BulkLifecycleInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !input.confirmed || input.team_ids.is_empty() || input.team_ids.len() > 200 {
        return error(
            StatusCode::BAD_REQUEST,
            "confirm between 1 and 200 Paper Teams",
        );
    }
    let requested = match input.status.parse::<persistence::PaperStatus>() {
        Ok(
            value @ (persistence::PaperStatus::Active
            | persistence::PaperStatus::Frozen
            | persistence::PaperStatus::Archived),
        ) => value,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "bulk status must be active, frozen, or archived",
            );
        }
    };
    let mut results = Vec::with_capacity(input.team_ids.len());
    let mut succeeded = 0_usize;
    for team_id in input.team_ids {
        match state.v2.set_paper_team_status(team_id, requested).await {
            Ok(team) => {
                succeeded += 1;
                results.push(serde_json::json!({"team_id":team_id,"ok":true,"status":team.status}));
            }
            Err(error_value) => results.push(serde_json::json!({
                "team_id":team_id,"ok":false,"error":error_value.to_string()
            })),
        }
    }
    let failed = results.len().saturating_sub(succeeded);
    if let Err(error_value) = state
        .institution
        .audit_bulk_lifecycle(principal.user_id(), requested.as_str(), succeeded, failed)
        .await
    {
        return institution_error(error_value);
    }
    Json(serde_json::json!({"succeeded":succeeded,"failed":failed,"results":results}))
        .into_response()
}

#[derive(Default)]
struct InstitutionUpload {
    filename: Option<String>,
    target_table: Option<String>,
    mode: Option<String>,
    bytes: Bytes,
}

async fn institution_upload_fields(
    mut multipart: Multipart,
) -> Result<InstitutionUpload, Response> {
    let mut upload = InstitutionUpload::default();
    let mut has_file = false;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid institution upload"))?
    {
        match field.name() {
            Some("file") if !has_file => {
                upload.filename = field.file_name().map(str::to_owned);
                upload.bytes = field
                    .bytes()
                    .await
                    .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid institution upload"))?;
                has_file = true;
            }
            Some("file") => {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "only one institution file may be uploaded",
                ));
            }
            Some("mode") => {
                upload.mode = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid import mode"))?,
                );
            }
            Some("target_table") => {
                upload.target_table = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid target table"))?,
                );
            }
            _ => {}
        }
    }
    if !has_file || upload.filename.is_none() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "multipart file with a filename is required",
        ));
    }
    Ok(upload)
}

async fn admin_v2_institution_validate(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let upload = match institution_upload_fields(multipart).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mode = match upload
        .mode
        .as_deref()
        .unwrap_or("VALIDATE_ONLY")
        .parse::<ImportMode>()
    {
        Ok(value) => value,
        Err(error_value) => return institution_error(error_value),
    };
    match state
        .institution
        .validate_upload(
            principal.user_id(),
            upload.filename.as_deref().unwrap_or_default(),
            upload
                .target_table
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty()),
            mode,
            &upload.bytes,
            ImportLimits::default(),
        )
        .await
    {
        Ok(job) => (StatusCode::CREATED, Json(job)).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

#[derive(Default)]
struct InstitutionBatchUploadFields {
    operation: Option<String>,
    uploads: Vec<InstitutionBatchUpload>,
}

async fn institution_batch_upload_fields(
    mut multipart: Multipart,
) -> Result<InstitutionBatchUploadFields, Response> {
    let mut fields = InstitutionBatchUploadFields::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid institution batch upload"))?
    {
        match field.name() {
            Some("operation") => {
                fields.operation = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid operation"))?,
                );
            }
            Some("files[]" | "file") => {
                let filename = field.file_name().map(str::to_owned).ok_or_else(|| {
                    error(
                        StatusCode::BAD_REQUEST,
                        "every uploaded file needs a filename",
                    )
                })?;
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid uploaded file"))?;
                fields.uploads.push(InstitutionBatchUpload {
                    filename,
                    target_table: None,
                    bytes: bytes.to_vec(),
                });
            }
            Some("target_table") => {
                let target = field
                    .text()
                    .await
                    .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid target dataset"))?;
                let Some(upload) = fields.uploads.last_mut() else {
                    return Err(error(
                        StatusCode::BAD_REQUEST,
                        "target dataset must follow its file",
                    ));
                };
                upload.target_table = Some(target);
            }
            _ => {}
        }
    }
    if fields.uploads.is_empty() {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "select at least one CSV or XLSX file",
        ));
    }
    Ok(fields)
}

async fn admin_v2_institution_batch_validate(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let fields = match institution_batch_upload_fields(multipart).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let operation = match fields
        .operation
        .as_deref()
        .unwrap_or("ADD")
        .parse::<InstitutionOperation>()
    {
        Ok(value) => value,
        Err(error_value) => return institution_error(error_value),
    };
    match state
        .institution
        .validate_batch(
            principal.user_id(),
            operation,
            &fields.uploads,
            ImportLimits::default(),
        )
        .await
    {
        Ok(batch) => (StatusCode::CREATED, Json(batch)).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_institution_batch_apply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(batch_id): Path<uuid::Uuid>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let detail = match state
        .institution
        .apply_batch(batch_id, principal.user_id())
        .await
    {
        Ok(value) => value,
        Err(error_value) => return institution_error(error_value),
    };
    let account_provisioning = detail["account_provisioning"].clone();
    if detail["batch"]["operation"] != "DELETE" {
        let job_id = match state.institution.batch_primary_job(batch_id).await {
            Ok(value) => value,
            Err(error_value) => return institution_error(error_value),
        };
        let materialization =
            admin_v2_institution_apply(State(state.clone()), headers.clone(), Path(job_id)).await;
        if materialization.status().is_server_error() {
            return materialization;
        }
    }
    match state.institution.batch_detail(batch_id, 100).await {
        Ok(mut current) => {
            current["account_provisioning"] = account_provisioning;
            Json(current).into_response()
        }
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_institution_batches(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ImportListQuery>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    let filter = ImportJobPageFilter {
        limit: query.limit.unwrap_or(25),
        page: query.page.unwrap_or(1),
        search: query.search,
        status: query.status,
        mode: query.mode,
        file_type: None,
    };
    match state.institution.paginated_batches(&filter).await {
        Ok(page) => Json(page).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_institution_batch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(batch_id): Path<uuid::Uuid>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    match state.institution.batch_detail(batch_id, 100).await {
        Ok(batch) => Json(batch).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "per-Team isolation keeps resolution, template cloning, and unresolved provenance adjacent"
)]
async fn admin_v2_institution_apply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<uuid::Uuid>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let applied_job = match state
        .institution
        .apply_import(job_id, principal.user_id())
        .await
    {
        Ok(value) => value,
        Err(error_value) => return institution_error(error_value),
    };
    let plans = match state.institution.pending_team_plans(job_id).await {
        Ok(value) => value,
        Err(error_value) => return institution_error(error_value),
    };
    let mut materialized = 0_u64;
    let mut merged = 0_u64;
    let mut unresolved = 0_u64;
    for plan in plans {
        if plan.existing_paper_team_id.is_some() {
            if applied_job.job.mode == "ADD_ONLY" {
                continue;
            }
            if !plan.unresolved.is_empty() {
                unresolved += 1;
                if let Err(error_value) = state
                    .institution
                    .mark_team_unresolved(job_id, &plan.external_team_key, &plan.unresolved)
                    .await
                {
                    return institution_error(error_value);
                }
                continue;
            }
            match state
                .institution
                .merge_existing_team(principal.user_id(), &plan)
                .await
            {
                Ok(()) => merged += 1,
                Err(error_value) => {
                    unresolved += 1;
                    let reasons = vec![error_value.to_string()];
                    if let Err(mark_error) = state
                        .institution
                        .mark_team_unresolved(job_id, &plan.external_team_key, &reasons)
                        .await
                    {
                        return institution_error(mark_error);
                    }
                }
            }
            continue;
        }
        if !plan.unresolved.is_empty() {
            unresolved += 1;
            if let Err(error_value) = state
                .institution
                .mark_team_unresolved(job_id, &plan.external_team_key, &plan.unresolved)
                .await
            {
                return institution_error(error_value);
            }
            continue;
        }
        let writers = plan
            .writer_user_ids
            .iter()
            .copied()
            .map(UserId::from_uuid)
            .collect::<Vec<_>>();
        let mentors = plan
            .mentor_user_ids
            .iter()
            .copied()
            .map(UserId::from_uuid)
            .collect::<Vec<_>>();
        let Some(leader) = plan.leader_user_id.map(UserId::from_uuid) else {
            unresolved += 1;
            let reasons = vec!["MISSING_LEADER".to_owned()];
            let _ = state
                .institution
                .mark_team_unresolved(job_id, &plan.external_team_key, &reasons)
                .await;
            continue;
        };
        let resolution = match state
            .institution
            .resolve_default_template_for_writers(&writers)
            .await
        {
            Ok(value) => value,
            Err(error_value) => {
                unresolved += 1;
                let reasons = vec![error_value.to_string()];
                let _ = state
                    .institution
                    .mark_team_unresolved(job_id, &plan.external_team_key, &reasons)
                    .await;
                continue;
            }
        };
        let (main_path, seeds, source_identity) =
            match template_seeds(&state, resolution.selected_template_id).await {
                Ok(value) => value,
                Err(response) => {
                    unresolved += 1;
                    let reasons = vec!["selected template is not materializable".to_owned()];
                    let _ = state
                        .institution
                        .mark_team_unresolved(job_id, &plan.external_team_key, &reasons)
                        .await;
                    tracing::warn!(external_team_key=%plan.external_team_key, "{response:?}");
                    continue;
                }
            };
        let imported = TeamTemplateResolutionInput {
            dominant_programme_code: resolution.dominant_programme_code.clone(),
            resolution_method: resolution.resolution_method.clone(),
            external_team_key: Some(plan.external_team_key.clone()),
            source_import_job_id: Some(job_id),
        };
        match state
            .v2
            .create_template_paper_team(
                principal.user_id(),
                principal.session.tenant_id,
                WorkspaceId::new(),
                &plan.team_name,
                leader,
                &writers,
                &mentors,
                resolution.selected_template_id,
                &source_identity,
                &main_path,
                &seeds,
                Some(&imported),
            )
            .await
        {
            Ok((_team, _)) => {
                materialized += 1;
            }
            Err(error_value) => {
                unresolved += 1;
                let reasons = vec![error_value.to_string()];
                if let Err(mark_error) = state
                    .institution
                    .mark_team_unresolved(job_id, &plan.external_team_key, &reasons)
                    .await
                {
                    return institution_error(mark_error);
                }
            }
        }
    }
    match state.institution.job(job_id).await {
        Ok(job) => Json(serde_json::json!({
            "job": job,
            "account_provisioning": applied_job.account_provisioning,
            "materialized_teams": materialized,
            "merged_teams": merged,
            "unresolved_teams": unresolved,
        }))
        .into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "retry keeps each idempotent imported-Team outcome explicit and independently reportable"
)]
async fn admin_v2_institution_retry_teams(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<uuid::Uuid>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let job = match state.institution.job(job_id).await {
        Ok(value) => value,
        Err(error_value) => return institution_error(error_value),
    };
    if !matches!(job.status.as_str(), "PARTIAL" | "APPLIED") {
        return error(
            StatusCode::CONFLICT,
            "only an applied import with unresolved Teams can be retried",
        );
    }
    let plans = match state.institution.pending_team_plans(job_id).await {
        Ok(value) => value,
        Err(error_value) => return institution_error(error_value),
    };
    let mut results = Vec::new();
    for plan in plans {
        if !plan.unresolved.is_empty() {
            results.push(serde_json::json!({"external_team_key":plan.external_team_key,"ok":false,"errors":plan.unresolved}));
            continue;
        }
        let result: Result<(), String> = if plan.existing_paper_team_id.is_some() {
            state
                .institution
                .merge_existing_team(principal.user_id(), &plan)
                .await
                .map_err(|error_value| error_value.to_string())
        } else {
            let writers = plan
                .writer_user_ids
                .iter()
                .copied()
                .map(UserId::from_uuid)
                .collect::<Vec<_>>();
            let mentors = plan
                .mentor_user_ids
                .iter()
                .copied()
                .map(UserId::from_uuid)
                .collect::<Vec<_>>();
            match plan.leader_user_id.map(UserId::from_uuid) {
                None => Err("Missing Team Leader".into()),
                Some(leader) => match state
                    .institution
                    .resolve_default_template_for_writers(&writers)
                    .await
                {
                    Err(error_value) => Err(error_value.to_string()),
                    Ok(resolution) => {
                        match template_seeds(&state, resolution.selected_template_id).await {
                            Err(_) => Err("selected template is not materializable".into()),
                            Ok((main_path, seeds, source_identity)) => {
                                let imported = TeamTemplateResolutionInput {
                                    dominant_programme_code: resolution.dominant_programme_code,
                                    resolution_method: resolution.resolution_method,
                                    external_team_key: Some(plan.external_team_key.clone()),
                                    source_import_job_id: Some(job_id),
                                };
                                state
                                    .v2
                                    .create_template_paper_team(
                                        principal.user_id(),
                                        principal.session.tenant_id,
                                        WorkspaceId::new(),
                                        &plan.team_name,
                                        leader,
                                        &writers,
                                        &mentors,
                                        resolution.selected_template_id,
                                        &source_identity,
                                        &main_path,
                                        &seeds,
                                        Some(&imported),
                                    )
                                    .await
                                    .map(|_| ())
                                    .map_err(|error_value| error_value.to_string())
                            }
                        }
                    }
                },
            }
        };
        match result {
            Ok(()) => {
                if let Err(error_value) = state.institution.mark_team_resolved(principal.user_id(), job_id, &plan.external_team_key).await { return institution_error(error_value); }
                results.push(serde_json::json!({"external_team_key":plan.external_team_key,"ok":true}));
            }
            Err(message) => results.push(serde_json::json!({"external_team_key":plan.external_team_key,"ok":false,"errors":[message]})),
        }
    }
    let succeeded = results.iter().filter(|result| result["ok"] == true).count();
    Json(serde_json::json!({"job_id":job_id,"succeeded":succeeded,"failed":results.len().saturating_sub(succeeded),"results":results})).into_response()
}

async fn admin_v2_institution_imports(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ImportListQuery>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    let uses_page_contract = query.page.is_some()
        || query.search.is_some()
        || query.status.is_some()
        || query.mode.is_some()
        || query.file_type.is_some();
    if !uses_page_contract {
        return match state
            .institution
            .list_jobs(query.limit.unwrap_or(50), query.before)
            .await
        {
            Ok(jobs) => Json(jobs).into_response(),
            Err(error_value) => institution_error(error_value),
        };
    }
    let filter = ImportJobPageFilter {
        limit: query.limit.unwrap_or(50),
        page: query.page.unwrap_or(1),
        search: query.search,
        status: query.status,
        mode: query.mode,
        file_type: query.file_type,
    };
    match state.institution.paginated_jobs(&filter).await {
        Ok(page) => Json(page).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_institution_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<uuid::Uuid>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    match state.institution.job_detail(job_id, 100).await {
        Ok(detail) => Json(detail).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

fn institution_page_filter(query: InstitutionDirectoryQuery) -> InstitutionPageFilter {
    InstitutionPageFilter {
        limit: query.limit.unwrap_or(50),
        page: query.page.unwrap_or(1),
        search: query.search,
        programme_code: query.programme_code,
        department_id: query.department_id,
        link_status: query.link_status,
        external_type: query.external_type,
    }
}

async fn admin_v2_institution_students(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<InstitutionDirectoryQuery>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    match state
        .institution
        .paginated_students(&institution_page_filter(query))
        .await
    {
        Ok(page) => Json(page).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_institution_faculty(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<InstitutionDirectoryQuery>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    match state
        .institution
        .paginated_faculty(&institution_page_filter(query))
        .await
    {
        Ok(page) => Json(page).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_institution_programmes(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<InstitutionDirectoryQuery>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    match state
        .institution
        .paginated_programmes(&institution_page_filter(query))
        .await
    {
        Ok(page) => Json(page).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_institution_dataset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(dataset): Path<String>,
    Query(query): Query<InstitutionDirectoryQuery>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    match state
        .institution
        .paginated_dataset(&dataset, &institution_page_filter(query))
        .await
    {
        Ok(page) => Json(page).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_institution_manual_validate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(dataset): Path<String>,
    Json(input): Json<ManualInstitutionOperationInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let operation = match input.operation.parse::<InstitutionOperation>() {
        Ok(value) => value,
        Err(error_value) => return institution_error(error_value),
    };
    match state
        .institution
        .validate_manual_operation(principal.user_id(), &dataset, operation, &input.payload)
        .await
    {
        Ok(detail) => (StatusCode::CREATED, Json(detail)).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_identity_links(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<InstitutionDirectoryQuery>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    match state
        .institution
        .paginated_identity_links(&institution_page_filter(query))
        .await
    {
        Ok(page) => Json(page).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_manual_identity_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ManualIdentityLinkInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .institution
        .manual_link_identity(
            principal.user_id(),
            &input.external_type,
            &input.external_id,
            input.user_id,
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_unlink_identity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((external_type, external_id)): Path<(String, String)>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .institution
        .unlink_identity(principal.user_id(), &external_type, &external_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_institution_user_search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<V2UserSearchQuery>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    match state
        .institution
        .search_v2_users(
            query.q.as_deref().unwrap_or(""),
            query.role.as_deref(),
            query.limit.unwrap_or(20),
        )
        .await
    {
        Ok(users) => Json(users).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_institution_errors(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<uuid::Uuid>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    let rows = match state.institution.job_rows(job_id, true).await {
        Ok(value) => value,
        Err(error_value) => return institution_error(error_value),
    };
    let mut body =
        String::from("source_table_or_sheet,row_number,natural_key,error_code,error_message\n");
    for row in rows {
        let _ = writeln!(
            body,
            "{},{},{},{},{}",
            csv_escape(&row.source_table_or_sheet),
            row.row_number,
            csv_escape(&row.natural_key.to_string()),
            csv_escape(row.error_code.as_deref().unwrap_or("")),
            csv_escape(row.error_message.as_deref().unwrap_or("")),
        );
    }
    let mut response = body.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=errors.csv"),
    );
    response
}

async fn admin_v2_programme_template_defaults(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    match state.institution.list_programme_template_defaults().await {
        Ok(value) => Json(value).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_set_programme_template_default(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(programme_code): Path<String>,
    Json(input): Json<ProgrammeTemplateInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .institution
        .set_programme_template_default(principal.user_id(), &programme_code, input.template_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_delete_programme_template_default(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(programme_code): Path<String>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .institution
        .delete_programme_template_default(principal.user_id(), &programme_code)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_template_resolve_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<TemplateResolvePreviewInput>,
) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    let writers = match parse_user_ids(&input.ordered_writer_user_ids) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .institution
        .resolve_default_template_for_writers(&writers)
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_global_fallback(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = institution_admin(&state, &headers).await {
        return response;
    }
    match state.institution.global_fallback_detail().await {
        Ok(detail) => Json(detail).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_set_global_fallback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ProgrammeTemplateInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .institution
        .set_global_fallback(principal.user_id(), input.template_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

async fn admin_v2_paper_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match (
        state.v2.paper_team(id).await,
        state.v2.list_paper_team_member_views(id).await,
        state.v2.paper_template_pin(principal.user_id(), id).await,
        state.institution.paper_team_admin_summary(id).await,
    ) {
        (Ok(team), Ok(members), Ok(template_pin), Ok(summary)) => {
            Json(serde_json::json!({"team":team,"members":members,"template_pin":template_pin,"summary":summary}))
                .into_response()
        }
        (Err(error_value), _, _, _) | (_, Err(error_value), _, _) | (_, _, Err(error_value), _) => {
            v2_error(error_value)
        }
        (_, _, _, Err(error_value)) => institution_error(error_value),
    }
}

async fn admin_v2_update_paper_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<V2PaperTeamUpdateInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let writers = match parse_user_ids(&input.writer_ids) {
        Ok(value) => value
            .into_iter()
            .map(|value| *value.as_uuid())
            .collect::<Vec<_>>(),
        Err(response) => return response,
    };
    let mentors = match parse_user_ids(&input.mentor_ids) {
        Ok(value) => value
            .into_iter()
            .map(|value| *value.as_uuid())
            .collect::<Vec<_>>(),
        Err(response) => return response,
    };
    let leader = match parse_user_id(&input.leader_writer_id) {
        Ok(value) => *value.as_uuid(),
        Err(response) => return response,
    };
    match state
        .institution
        .update_paper_team(
            principal.user_id(),
            id,
            &input.name,
            &writers,
            leader,
            &mentors,
        )
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error_value) => institution_error(error_value),
    }
}

struct PreparedTemplateChange {
    response: serde_json::Value,
    token: String,
    request: TemplateChangeRequest,
    document_epoch: u64,
    blocking_conflicts: usize,
}

#[allow(
    clippy::too_many_lines,
    reason = "exact-state conflict classification stays adjacent so preview and apply share one safety decision"
)]
async fn prepare_template_change(
    state: &AppState,
    admin: UserId,
    paper_id: uuid::Uuid,
    input: &TemplateChangeInput,
) -> Result<PreparedTemplateChange, Response> {
    let team = state.v2.paper_team(paper_id).await.map_err(v2_error)?;
    if team.status == persistence::PaperStatus::Archived {
        return Err(error(
            StatusCode::CONFLICT,
            "archived Paper Team template cannot be changed",
        ));
    }
    let pin = state
        .v2
        .paper_template_pin(admin, paper_id)
        .await
        .map_err(v2_error)?
        .ok_or_else(|| error(StatusCode::CONFLICT, "Paper Team has no pinned template"))?;
    let old_template_id = pin
        .get("template_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .ok_or_else(|| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid current template pin",
            )
        })?;
    if old_template_id == input.new_template_id {
        return Err(error(
            StatusCode::CONFLICT,
            "selected template is already pinned",
        ));
    }
    let paper = persistence::WriterPaper {
        id: team.id,
        workspace_id: team.workspace_id,
        name: team.name.clone(),
        kind: persistence::PaperKind::Team,
        status: team.status,
        is_team_leader: false,
        updated_at: team.updated_at.clone(),
    };
    let exact = capture_exact_v2_state(state, &paper).await?;
    let workspace_manifest: WorkspaceManifestV1 =
        serde_json::from_value(exact.manifest.get("workspace").cloned().ok_or_else(|| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "exact state has no workspace manifest",
            )
        })?)
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "exact workspace manifest is invalid",
            )
        })?;
    let old_files = state
        .repo
        .template_files(old_template_id)
        .await
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "current template lookup failed",
            )
        })?;
    let old_by_path = old_files
        .into_iter()
        .map(|file| (file.path, file.blob_hash))
        .collect::<BTreeMap<_, _>>();
    let (new_main, new_seeds, new_source_identity) =
        template_seeds(state, input.new_template_id).await?;
    let live_files = state
        .v2
        .list_live_paper_files(team.workspace_id)
        .await
        .map_err(v2_error)?;
    let live_by_path = live_files
        .into_iter()
        .map(|file| (file.path.as_str().to_owned(), file))
        .collect::<BTreeMap<_, _>>();
    let mut files_to_add = Vec::new();
    let mut managed_updates = Vec::new();
    let mut unchanged_template_updates = Vec::new();
    let mut unchanged_files = Vec::new();
    let mut conflicts = Vec::new();
    let mut mutations = Vec::new();
    for seed in new_seeds {
        let path = seed.path.as_str().to_owned();
        let current_entry = workspace_manifest.files().get(&seed.path);
        let existing = live_by_path.get(&path);
        match current_entry {
            None => {
                files_to_add.push(path.clone());
                mutations.push(TemplateChangeFile {
                    path: seed.path,
                    blob_hash: seed.blob_hash,
                    size_bytes: seed.size_bytes,
                    policy: seed.policy,
                    existing_file_id: None,
                });
            }
            Some(current) if current.blob_hash == seed.blob_hash => unchanged_files.push(path),
            Some(current) => {
                let policy = if let Some(file) = existing {
                    state.v2.file_policy(file.file_id).await.map_err(v2_error)?
                } else {
                    conflicts.push(serde_json::json!({"path":path,"reason":"workspace path has no stable Paper file identity"}));
                    continue;
                };
                let unchanged_from_old = old_by_path
                    .get(&path)
                    .is_some_and(|hash| *hash == current.blob_hash);
                if policy == V2FilePolicy::TemplateManaged || unchanged_from_old {
                    if policy == V2FilePolicy::TemplateManaged {
                        managed_updates.push(path.clone());
                    } else {
                        unchanged_template_updates.push(path.clone());
                    }
                    mutations.push(TemplateChangeFile {
                        path: seed.path,
                        blob_hash: seed.blob_hash,
                        size_bytes: seed.size_bytes,
                        policy: seed.policy,
                        existing_file_id: existing.map(|file| file.file_id),
                    });
                } else {
                    conflicts.push(serde_json::json!({"path":path,"reason":"Writer-created or Writer-modified content occupies the new template path"}));
                }
            }
        }
    }
    for path in workspace_manifest.files().keys() {
        if !mutations.iter().any(|file| &file.path == path)
            && !unchanged_files.iter().any(|value| value == path.as_str())
        {
            unchanged_files.push(path.as_str().to_owned());
        }
    }
    let main_changed = workspace_manifest.main_file() != &new_main;
    if main_changed && !input.confirm_main_file_change {
        conflicts.push(serde_json::json!({"path":new_main,"reason":"Main file change requires explicit confirmation"}));
    }
    let token = digest(&format!(
        "{paper_id}:{}:{old_template_id}:{}:{}",
        exact.state_hash, input.new_template_id, input.confirm_main_file_change
    ));
    let blocking_conflicts = conflicts.len();
    let request = TemplateChangeRequest {
        paper_id,
        workspace_id: team.workspace_id,
        expected_workspace_version: exact.source_sequence,
        expected_template_id: old_template_id,
        new_template_id: input.new_template_id,
        new_source_identity,
        new_main_file: main_changed.then_some(new_main.clone()),
        files: mutations,
        safety: ExactRestoreState {
            document_epoch: exact.document_epoch,
            workspace_version: exact.source_sequence,
            snapshot_id: exact.snapshot_id,
            manifest: exact.manifest.clone(),
            state_hash: exact.state_hash.clone(),
        },
    };
    let response = serde_json::json!({
        "paper_id":paper_id,"current_template_id":old_template_id,"new_template_id":input.new_template_id,
        "preview_token":token,"workspace_state_hash":exact.state_hash,
        "files_to_add":files_to_add,"template_managed_files_to_update":managed_updates,
        "unchanged_old_template_files_to_update":unchanged_template_updates,"unchanged_files":unchanged_files,
        "writer_modified_conflicts":conflicts,"blocking_conflicts":blocking_conflicts,
        "main_file_change":{"changed":main_changed,"from":workspace_manifest.main_file(),"to":new_main,"confirmed":input.confirm_main_file_change},
        "package_preamble_notes":["Template replacement is whole-file only; no per-line merge is attempted.","Files absent from the new template are preserved."],
        "can_apply":blocking_conflicts == 0,
    });
    Ok(PreparedTemplateChange {
        response,
        token,
        request,
        document_epoch: exact.document_epoch,
        blocking_conflicts,
    })
}

async fn admin_v2_template_change_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<TemplateChangeInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match prepare_template_change(&state, principal.user_id(), id, &input).await {
        Ok(prepared) => {
            if let Err(error_value) = state
                .institution
                .audit_template_preview(
                    principal.user_id(),
                    id,
                    input.new_template_id,
                    prepared.blocking_conflicts,
                )
                .await
            {
                return institution_error(error_value);
            }
            Json(prepared.response).into_response()
        }
        Err(response) => response,
    }
}

async fn admin_v2_template_change_apply(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<TemplateChangeInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match institution_admin(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let supplied_token = match input.preview_token.as_deref() {
        Some(value) if value.len() == 64 => value,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "a valid template-change preview token is required",
            );
        }
    };
    let prepared = match prepare_template_change(&state, principal.user_id(), id, &input).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if supplied_token != prepared.token {
        return error(
            StatusCode::CONFLICT,
            "Paper Team state changed after preview; preview again",
        );
    }
    if prepared.blocking_conflicts != 0 {
        return error(
            StatusCode::CONFLICT,
            "template change conflicts with Writer files",
        );
    }
    let next = match state
        .v2
        .apply_template_change(principal.user_id(), &prepared.request)
        .await
    {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    let team = match state.v2.paper_team(id).await {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    let paper = persistence::WriterPaper {
        id: team.id,
        workspace_id: team.workspace_id,
        name: team.name,
        kind: persistence::PaperKind::Team,
        status: team.status,
        is_team_leader: false,
        updated_at: team.updated_at,
    };
    let exact = match capture_exact_v2_state(&state, &paper).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if exact.source_sequence != next {
        return error(
            StatusCode::CONFLICT,
            "workspace changed while finalizing template update",
        );
    }
    let version_id = match state
        .v2
        .finalize_template_change(
            principal.user_id(),
            id,
            paper.workspace_id,
            input.new_template_id,
            ExactRestoreState {
                document_epoch: exact.document_epoch,
                workspace_version: exact.source_sequence,
                snapshot_id: exact.snapshot_id,
                manifest: exact.manifest,
                state_hash: exact.state_hash,
            },
        )
        .await
    {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    state
        .collaboration
        .epoch_changed(paper.workspace_id, prepared.document_epoch)
        .await;
    Json(serde_json::json!({"paper_id":id,"template_id":input.new_template_id,"resolution_method":"MANUAL_OVERRIDE","workspace_version":next,"template_update_version_id":version_id,"safety_checkpoint":"PRE_TEMPLATE_CHANGE"})).into_response()
}

#[allow(clippy::too_many_lines)]
async fn admin_v2_create_paper_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<V2PaperTeamInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !matches!(principal.kind, PrincipalKind::V2(GlobalRole::Admin)) {
        return error(StatusCode::FORBIDDEN, "V2 Admin required for Paper Teams");
    }
    let writer_ids = match parse_user_ids(&input.writer_ids) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mentor_ids = match parse_user_ids(&input.mentor_ids) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let leader_writer_id = match parse_user_id(&input.leader_writer_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let automatic_resolution = if input.template_id.is_none() {
        match state
            .institution
            .resolve_default_template_for_writers(&writer_ids)
            .await
        {
            Ok(value) => Some(value),
            Err(error_value) => return institution_error(error_value),
        }
    } else {
        None
    };
    let template_id = input.template_id.unwrap_or_else(|| {
        automatic_resolution
            .as_ref()
            .expect("automatic resolution is present")
            .selected_template_id
    });
    let (main_path, seeds, source_identity) = match template_seeds(&state, template_id).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let resolution_input = automatic_resolution.as_ref().map_or_else(
        || TeamTemplateResolutionInput {
            dominant_programme_code: None,
            resolution_method: "MANUAL_OVERRIDE".to_owned(),
            external_team_key: None,
            source_import_job_id: None,
        },
        |resolution| TeamTemplateResolutionInput {
            dominant_programme_code: resolution.dominant_programme_code.clone(),
            resolution_method: resolution.resolution_method.clone(),
            external_team_key: None,
            source_import_job_id: None,
        },
    );
    match state
        .v2
        .create_template_paper_team(
            principal.user_id(),
            principal.session.tenant_id,
            WorkspaceId::new(),
            &input.name,
            leader_writer_id,
            &writer_ids,
            &mentor_ids,
            template_id,
            &source_identity,
            &main_path,
            &seeds,
            Some(&resolution_input),
        )
        .await
    {
        Ok((team, files)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "team":team,"files":files,
                "template_pin":{"template_id":template_id,"source_identity":source_identity},
                "template_resolution":automatic_resolution,
                "manual_override":input.template_id.is_some(),
            })),
        )
            .into_response(),
        Err(value) => v2_error(value),
    }
}

async fn admin_v2_change_paper_team_leader(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<V2PaperTeamMemberInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let leader_writer_id = match parse_user_id(&input.user_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .change_paper_team_leader(id, leader_writer_id, principal.user_id())
        .await
    {
        Ok(member) => Json(member).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn admin_v2_add_paper_team_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<V2PaperTeamMemberInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let user_id = match parse_user_id(&input.user_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .add_paper_team_member(id, user_id, principal.user_id())
        .await
    {
        Ok(member) => (StatusCode::CREATED, Json(member)).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn admin_v2_remove_paper_team_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, user_id)): Path<(uuid::Uuid, String)>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let user_id = match parse_user_id(&user_id) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .remove_paper_team_member(id, user_id, principal.user_id())
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn admin_v2_paper_team_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<V2StatusInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !matches!(principal.kind, PrincipalKind::V2(GlobalRole::Admin)) {
        return error(
            StatusCode::FORBIDDEN,
            "V2 Admin required for Paper Team governance",
        );
    }
    let current = match state.v2.paper_team(id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let wanted = match persistence::PaperStatus::from_str(&input.status) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid Paper Team status"),
    };
    let legal = matches!(
        (current.status, wanted),
        (
            persistence::PaperStatus::Active,
            persistence::PaperStatus::Frozen
                | persistence::PaperStatus::Submitted
                | persistence::PaperStatus::Archived
        ) | (
            persistence::PaperStatus::Frozen,
            persistence::PaperStatus::Active | persistence::PaperStatus::Archived
        ) | (
            persistence::PaperStatus::Submitted,
            persistence::PaperStatus::Archived
        )
    );
    if !legal {
        return error(
            StatusCode::CONFLICT,
            "invalid Paper Team lifecycle transition",
        );
    }
    match state.v2.set_paper_team_status(id, wanted).await {
        Ok(team) => Json(team).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn admin_v2_file_policies(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.v2.admin_file_policies(principal.user_id(), id).await {
        Ok(files) => Json(files).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn admin_v2_set_file_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, file_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(input): Json<FilePolicyInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let policy = match V2FilePolicy::parse(&input.policy) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid V2 file policy"),
    };
    match state
        .v2
        .set_file_policy(principal.user_id(), id, file_id, policy)
        .await
    {
        Ok(record) => {
            state
                .collaboration
                .policy_changed(record.workspace_id, record.file_id)
                .await;
            Json(record).into_response()
        }
        Err(value) => v2_error(value),
    }
}

async fn admin_v2_restoration_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .restoration_requests_for_actor(principal.user_id(), None)
        .await
    {
        Ok(requests) => Json(requests).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn gone_restoration_governance() -> Response {
    error(
        StatusCode::GONE,
        "Mentor/Admin restoration governance is deprecated; Team reverts are controlled by the Team Leader.",
    )
}

async fn admin_v2_versions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.v2.admin_versions(principal.user_id()).await {
        Ok(versions) => Json(versions).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn admin_v2_reviews(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.v2.admin_reviews(principal.user_id()).await {
        Ok(reviews) => Json(reviews).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn writer_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedPrincipal, Response> {
    let principal = principal_auth(state, headers).await?;
    if !matches!(principal.kind, PrincipalKind::V2(GlobalRole::Writer)) {
        return Err(error(StatusCode::FORBIDDEN, "V2 Writer required"));
    }
    Ok(principal)
}

async fn v2_paper_reader_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedPrincipal, Response> {
    let principal = principal_auth(state, headers).await?;
    if !matches!(
        principal.kind,
        PrincipalKind::V2(GlobalRole::Writer | GlobalRole::Mentor)
    ) {
        return Err(error(StatusCode::FORBIDDEN, "V2 Writer or Mentor required"));
    }
    Ok(principal)
}

async fn v2_writer_papers(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.v2.writer_papers(principal.user_id()).await {
        Ok(papers) => Json(papers).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_create_personal_paper(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<V2PaperInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let main_path = LogicalPath::parse("main.tex").expect("static main path is valid");
    let stored = match state
        .blobs
        .put(Bytes::from_static(INITIAL_TEX.as_bytes()))
        .await
    {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "blob storage failure"),
    };
    match state
        .v2
        .create_initialized_personal_paper(
            principal.user_id(),
            principal.session.tenant_id,
            WorkspaceId::new(),
            &input.name,
            &main_path,
            stored.hash(),
            stored.size_bytes(),
        )
        .await
    {
        Ok((paper, file)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"paper":paper,"main_file":file,"version":1})),
        )
            .into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_paper(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
) -> Response {
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let paper = match state.v2.writer_paper(principal.user_id(), paper_id).await {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    match state.workspaces.restore(paper.workspace_id).await {
        Ok(workspace) => Json(serde_json::json!({
            "paper":paper,
            "version":workspace.version().get(),
            "main_file":workspace.main_file().map(LogicalPath::as_str),
            "editable":paper.status == persistence::PaperStatus::Active
        }))
        .into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"),
    }
}

async fn v2_paper_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
) -> Response {
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let paper = match state.v2.writer_paper(principal.user_id(), paper_id).await {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    match state.v2.visible_paper_files(paper.workspace_id).await {
        Ok(files) => {
            let mut result = Vec::with_capacity(files.len());
            for file in files {
                let policy = match state.v2.file_policy(file.file_id).await {
                    Ok(value) => value,
                    Err(error_value) => return v2_error(error_value),
                };
                result.push(serde_json::json!({
                    "file_id":file.file_id,"workspace_id":file.workspace_id,"path":file.path,
                    "revision":file.revision,"tombstoned":file.tombstoned,"created_at":file.created_at,
                    "updated_at":file.updated_at,"tombstoned_at":file.tombstoned_at,"policy":policy
                }));
            }
            Json(result).into_response()
        }
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, file_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Response {
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (paper, file) = match authorized_file(&state, principal.user_id(), paper_id, file_id).await
    {
        Ok(value) => value,
        Err(response) => return response,
    };
    let workspace = match state.workspaces.restore(paper.workspace_id).await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"),
    };
    let bytes = match state
        .workspaces
        .read_file(paper.workspace_id, &file.path)
        .await
    {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"),
    };
    let content = match String::from_utf8(bytes.to_vec()) {
        Ok(value) => value,
        Err(_) => {
            return error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "binary file is not editable",
            );
        }
    };
    let policy = match state.v2.file_policy(file_id).await {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    Json(serde_json::json!({
        "file":file,
        "content":content,
        "version":workspace.version().get(),
        "main":workspace.main_file() == Some(&file.path),
        "editable":paper.status == persistence::PaperStatus::Active && policy.content_editable(),
        "policy":policy
    }))
    .into_response()
}

async fn v2_collaboration_socket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, file_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    websocket: WebSocketUpgrade,
) -> Response {
    let principal = match principal_auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !collaboration::websocket_principal_is_supported(&principal) {
        return error(
            StatusCode::FORBIDDEN,
            "paper collaboration participant required",
        );
    }
    let token = match collaboration::session_token(&headers) {
        Some(value) => value,
        None => return error(StatusCode::UNAUTHORIZED, "authentication required"),
    };
    let access = match state
        .v2
        .collaboration_access(principal.user_id(), paper_id, file_id)
        .await
    {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    let user_id = principal.user_id();
    websocket.on_upgrade(move |socket| {
        collaboration::serve_socket(state, socket, token, user_id, paper_id, access)
    })
}

async fn v2_create_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
    Json(input): Json<V2CreateFileInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let paper = match state.v2.writer_paper(principal.user_id(), paper_id).await {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    let path = match LogicalPath::parse(&input.path) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid file path"),
    };
    let stored = match state.blobs.put(Bytes::from(input.content)).await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "blob storage failure"),
    };
    match state
        .v2
        .create_file_with_event(
            paper.workspace_id,
            principal.user_id(),
            input.version,
            path,
            stored.hash(),
            stored.size_bytes(),
        )
        .await
    {
        Ok((file, version)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"file":file,"version":version})),
        )
            .into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_save_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, file_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(input): Json<V2SaveFileInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(response) = authorized_file(&state, principal.user_id(), paper_id, file_id).await {
        return response;
    }
    let stored = match state.blobs.put(Bytes::from(input.content)).await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "blob storage failure"),
    };
    match state
        .v2
        .save_file_with_event(
            file_id,
            principal.user_id(),
            input.version,
            stored.hash(),
            stored.size_bytes(),
        )
        .await
    {
        Ok((file, version)) => {
            Json(serde_json::json!({"file":file,"version":version})).into_response()
        }
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_rename_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, file_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(input): Json<V2RenameFileInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (paper, _) = match authorized_file(&state, principal.user_id(), paper_id, file_id).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if state
        .collaboration
        .flush_workspace(paper.workspace_id)
        .await
        .is_err()
    {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "collaboration flush failed",
        );
    }
    let path = match LogicalPath::parse(&input.path) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid file path"),
    };
    match state
        .v2
        .rename_file_with_event(file_id, principal.user_id(), input.version, path)
        .await
    {
        Ok((file, version)) => {
            Json(serde_json::json!({"file":file,"version":version})).into_response()
        }
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_delete_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, file_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(input): Json<V2VersionInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (paper, file) = match authorized_file(&state, principal.user_id(), paper_id, file_id).await
    {
        Ok(value) => value,
        Err(response) => return response,
    };
    if state
        .collaboration
        .flush_workspace(paper.workspace_id)
        .await
        .is_err()
    {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "collaboration flush failed",
        );
    }
    let workspace = match state.workspaces.restore(paper.workspace_id).await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"),
    };
    let Some(workspace_file) = workspace.file(&file.path) else {
        return error(
            StatusCode::CONFLICT,
            "file is absent from canonical workspace",
        );
    };
    match state
        .v2
        .delete_file_with_event(
            file_id,
            principal.user_id(),
            input.version,
            workspace_file.blob_hash(),
            workspace_file.size_bytes(),
            workspace.main_file() == Some(&file.path),
        )
        .await
    {
        Ok(version) => {
            state
                .collaboration
                .file_deleted(paper.workspace_id, file_id)
                .await;
            Json(serde_json::json!({"version":version})).into_response()
        }
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_set_main(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, file_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(input): Json<V2VersionInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (paper, _) = match authorized_file(&state, principal.user_id(), paper_id, file_id).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let previous_main = match state.workspaces.restore(paper.workspace_id).await {
        Ok(value) => value.main_file().cloned(),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"),
    };
    match state
        .v2
        .set_main_with_event(file_id, principal.user_id(), input.version, previous_main)
        .await
    {
        Ok(version) => Json(serde_json::json!({"version":version})).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_structural_undo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
) -> Response {
    v2_structural_history(state, headers, paper_id, false).await
}

async fn v2_structural_redo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
) -> Response {
    v2_structural_history(state, headers, paper_id, true).await
}

async fn v2_structural_history(
    state: AppState,
    headers: HeaderMap,
    paper_id: uuid::Uuid,
    redo: bool,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let paper = match state.v2.writer_paper(principal.user_id(), paper_id).await {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    if state
        .collaboration
        .flush_workspace(paper.workspace_id)
        .await
        .is_err()
    {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "collaboration flush failed",
        );
    }
    let result = if redo {
        state
            .v2
            .structural_redo(paper_id, paper.workspace_id, principal.user_id())
            .await
    } else {
        state
            .v2
            .structural_undo(paper_id, paper.workspace_id, principal.user_id())
            .await
    };
    match result {
        Ok(result) => {
            let tombstoned = (!redo && result.operation_type == "CREATE_FILE")
                || (redo && result.operation_type == "DELETE_FILE");
            if tombstoned {
                if let Some(file_id) = result.file_id {
                    state
                        .collaboration
                        .file_deleted(paper.workspace_id, file_id)
                        .await;
                }
            }
            Json(result).into_response()
        }
        Err(error_value) => v2_error(error_value),
    }
}

async fn canonical_paper_sources(
    state: &AppState,
    paper: &persistence::WriterPaper,
) -> Result<
    (
        workspace_model::WorkspaceState,
        Vec<persistence::PaperFile>,
        BTreeMap<LogicalPath, Bytes>,
    ),
    Response,
> {
    state
        .collaboration
        .flush_workspace(paper.workspace_id)
        .await
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "collaboration flush failed",
            )
        })?;
    let workspace = state
        .workspaces
        .restore(paper.workspace_id)
        .await
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"))?;
    let files = state
        .v2
        .visible_paper_files(paper.workspace_id)
        .await
        .map_err(v2_error)?;
    let mut sources = BTreeMap::new();
    for (path, entry) in workspace.files() {
        let bytes = state
            .blobs
            .get(entry.blob_hash())
            .await
            .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "paper blob unavailable"))?;
        sources.insert(path.clone(), bytes);
    }
    Ok((workspace, files, sources))
}

fn source_range_json(range: latex_parser::SourceRange) -> serde_json::Value {
    serde_json::json!({
        "start_byte":range.start_byte(),
        "end_byte":range.end_byte(),
        "start_line":range.start().row() + 1,
        "start_column":range.start().column_bytes(),
        "end_line":range.end().row() + 1,
        "end_column":range.end().column_bytes(),
    })
}

fn section_level_name(level: SectionLevel) -> &'static str {
    match level {
        SectionLevel::Part => "part",
        SectionLevel::Chapter => "chapter",
        SectionLevel::Section => "section",
        SectionLevel::Subsection => "subsection",
        SectionLevel::Subsubsection => "subsubsection",
        SectionLevel::Paragraph => "paragraph",
        SectionLevel::Subparagraph => "subparagraph",
    }
}

fn diagnostic_severity_name(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
        DiagnosticSeverity::Information => "information",
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "the normalized versioned intelligence response is assembled in one auditable adapter"
)]
async fn v2_paper_intelligence(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
) -> Response {
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let paper = match state.v2.writer_paper(principal.user_id(), paper_id).await {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    let (workspace, files, sources) = match canonical_paper_sources(&state, &paper).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let Some(main_file) = workspace.main_file().cloned() else {
        return error(StatusCode::CONFLICT, "paper has no main file");
    };
    let project = match ProjectSource::new(main_file, sources) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "invalid paper source"),
    };
    let analysis = match ProjectAnalyzer::with_default_limits().analyze(&project) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "paper analysis failed"),
    };
    let file_ids = files
        .iter()
        .map(|file| (file.path.clone(), file.file_id))
        .collect::<BTreeMap<_, _>>();
    let mut outline = Vec::new();
    let mut labels = Vec::new();
    let mut references = Vec::new();
    let mut citations = Vec::new();
    let mut environments = BTreeSet::new();
    let mut packages = BTreeSet::new();
    let known_labels = analysis
        .files()
        .values()
        .flat_map(|file| file.labels().iter().map(|label| label.key().to_owned()))
        .collect::<BTreeSet<_>>();
    let bibliography_keys = analysis
        .bibliographies()
        .values()
        .flat_map(|bibliography| {
            bibliography
                .entries()
                .iter()
                .map(|entry| entry.key().to_owned())
        })
        .collect::<BTreeSet<_>>();
    for (path, file) in analysis.files() {
        let Some(file_id) = file_ids.get(path) else {
            continue;
        };
        for section in file.sections() {
            outline.push(serde_json::json!({
                "level":section_level_name(section.level()),"title":section.title(),
                "file_id":file_id,"path":path,"range":source_range_json(section.range())
            }));
        }
        for label in file.labels() {
            labels.push(serde_json::json!({"key":label.key(),"file_id":file_id,"path":path,"range":source_range_json(label.range())}));
        }
        for reference in file.references() {
            references.push(serde_json::json!({
                "key":reference.key(),"resolved":known_labels.contains(reference.key()),
                "file_id":file_id,"path":path,"range":source_range_json(reference.range())
            }));
        }
        for citation in file.citations() {
            for key in citation.keys() {
                citations.push(serde_json::json!({
                    "key":key,"resolved":bibliography_keys.contains(key),"command":citation.command(),
                    "file_id":file_id,"path":path,"range":source_range_json(citation.range())
                }));
            }
        }
        environments.extend(
            file.environments()
                .iter()
                .map(|environment| environment.name().to_owned()),
        );
        packages.extend(
            file.packages()
                .iter()
                .map(|package| package.name().to_owned()),
        );
    }
    let bibliography = analysis
        .bibliographies()
        .iter()
        .flat_map(|(path, bibliography)| {
            let file_id = file_ids.get(path).copied();
            bibliography.entries().iter().map(move |entry| {
                let field = |wanted: &str| {
                    entry
                        .fields()
                        .iter()
                        .find(|field| field.name().eq_ignore_ascii_case(wanted))
                        .map(latex_parser::BibtexField::raw_value)
                };
                serde_json::json!({
                    "key":entry.key(),"entry_type":entry.entry_type(),"title":field("title"),
                    "author":field("author"),"file_id":file_id,"path":path,
                    "range":source_range_json(entry.range())
                })
            })
        })
        .collect::<Vec<_>>();
    let diagnostics = analysis
        .diagnostics()
        .iter()
        .map(|diagnostic| {
            serde_json::json!({
                "severity":diagnostic_severity_name(diagnostic.diagnostic().severity()),
                "code":format!("{:?}", diagnostic.diagnostic().code()),
                "message":diagnostic.diagnostic().message(),
                "file_id":file_ids.get(diagnostic.file()),"path":diagnostic.file(),
                "range":diagnostic.diagnostic().range().map(source_range_json),
            })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({
        "schema_version":1,"workspace_version":workspace.version().get(),"outline":outline,
        "labels":labels,"references":references,"citations":citations,
        "bibliography":bibliography,"environments":environments,"packages":packages,
        "diagnostics":diagnostics
    }))
    .into_response()
}

async fn v2_project_search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
    Query(query): Query<V2SearchQuery>,
) -> Response {
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let needle = query.q.trim();
    if needle.is_empty() || needle.len() > 200 {
        return error(
            StatusCode::BAD_REQUEST,
            "search query must contain 1-200 characters",
        );
    }
    let paper = match state.v2.writer_paper(principal.user_id(), paper_id).await {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    let (workspace, files, sources) = match canonical_paper_sources(&state, &paper).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let file_ids = files
        .iter()
        .map(|file| (&file.path, file.file_id))
        .collect::<BTreeMap<_, _>>();
    let comparison = if query.case_sensitive {
        needle.to_owned()
    } else {
        needle.to_lowercase()
    };
    let mut results = Vec::new();
    'files: for (path, bytes) in sources {
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        for (index, line) in text.lines().enumerate() {
            let haystack = if query.case_sensitive {
                line.to_owned()
            } else {
                line.to_lowercase()
            };
            for (column, _) in haystack.match_indices(&comparison) {
                results.push(serde_json::json!({
                    "file_id":file_ids.get(&path),"path":path,"line":index + 1,
                    "column":column,"preview":line.trim(),"match":needle
                }));
                if results.len() >= 200 {
                    break 'files;
                }
            }
        }
    }
    Json(serde_json::json!({"schema_version":1,"workspace_version":workspace.version().get(),"results":results,"truncated":results.len() == 200})).into_response()
}

async fn v2_upload_asset(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
    Query(query): Query<V2AssetQuery>,
    body: Bytes,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let paper = match state.v2.writer_paper(principal.user_id(), paper_id).await {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    let path = match LogicalPath::parse(&query.path) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid asset path"),
    };
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let extension = path.extension().unwrap_or("").to_ascii_lowercase();
    let valid = matches!(
        (extension.as_str(), content_type),
        ("png", "image/png")
            | ("jpg" | "jpeg", "image/jpeg")
            | ("pdf", "application/pdf")
            | ("csv", "text/csv" | "application/csv")
    );
    if !valid || body.is_empty() {
        return error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "asset must be a non-empty PNG, JPEG, PDF, or CSV matching its content type",
        );
    }
    let stored = match state.blobs.put(body).await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "blob storage failure"),
    };
    match state
        .v2
        .create_file_with_event(
            paper.workspace_id,
            principal.user_id(),
            query.version,
            path,
            stored.hash(),
            stored.size_bytes(),
        )
        .await
    {
        Ok((file, version)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"file":file,"version":version})),
        )
            .into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_raw_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, file_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Response {
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (paper, file) = match authorized_file(&state, principal.user_id(), paper_id, file_id).await
    {
        Ok(value) => value,
        Err(response) => return response,
    };
    let bytes = match state
        .workspaces
        .read_file(paper.workspace_id, &file.path)
        .await
    {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"),
    };
    let content_type = match file
        .path
        .extension()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        Some("csv") => "text/csv; charset=utf-8",
        _ => "application/octet-stream",
    };
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static(content_type)),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-store"),
            ),
        ],
        bytes,
    )
        .into_response()
}

#[derive(Clone)]
struct ExactV2State {
    document_epoch: u64,
    source_sequence: u64,
    snapshot_id: core_types::SnapshotId,
    manifest: serde_json::Value,
    state_hash: String,
}

async fn capture_exact_v2_state(
    state: &AppState,
    paper: &persistence::WriterPaper,
) -> Result<ExactV2State, Response> {
    state
        .collaboration
        .flush_workspace(paper.workspace_id)
        .await
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "collaboration flush failed",
            )
        })?;
    let document_epoch = state
        .v2
        .paper_document_epoch(paper.workspace_id)
        .await
        .map_err(v2_error)?;
    let checkpoint = state
        .workspaces
        .force_snapshot(paper.workspace_id)
        .await
        .map_err(|_| error(StatusCode::BAD_REQUEST, "paper must have a main file"))?;
    let bytes = state
        .blobs
        .get(checkpoint.manifest_blob_hash())
        .await
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "snapshot storage failure",
            )
        })?;
    let workspace_manifest: WorkspaceManifestV1 = serde_json::from_slice(&bytes).map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid snapshot manifest",
        )
    })?;
    let cutoffs = state
        .v2
        .collaboration_cutoffs(paper.workspace_id, document_epoch)
        .await
        .map_err(v2_error)?;
    let files = state
        .v2
        .list_live_paper_files(paper.workspace_id)
        .await
        .map_err(v2_error)?;
    let mut file_identities = Vec::with_capacity(files.len());
    let mut file_policies = Vec::with_capacity(files.len());
    for file in files {
        let policy = state.v2.file_policy(file.file_id).await.map_err(v2_error)?;
        file_identities.push(serde_json::json!({"file_id":file.file_id,"path":file.path}));
        file_policies.push(serde_json::json!({"file_id":file.file_id,"policy":policy}));
    }
    let manifest = serde_json::json!({
        "schema_version": 1,
        "paper_id": paper.id,
        "workspace_id": paper.workspace_id,
        "document_epoch": document_epoch,
        "source_sequence": checkpoint.workspace_version().get(),
        "workspace_snapshot_id": checkpoint.snapshot_id(),
        "collaboration_cutoffs": cutoffs.into_iter().map(|(file_id, sequence)| {
            serde_json::json!({"file_id":file_id,"durable_sequence":sequence})
        }).collect::<Vec<_>>(),
        "file_identities": file_identities,
        "workspace": workspace_manifest,
        "template_policy_provenance": {"file_policies":file_policies},
    });
    let state_hash = checkpoint.snapshot_id().to_hex();
    Ok(ExactV2State {
        document_epoch,
        source_sequence: checkpoint.workspace_version().get(),
        snapshot_id: checkpoint.snapshot_id(),
        manifest,
        state_hash,
    })
}

async fn v2_create_checkpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
    Json(input): Json<V2CheckpointInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let paper = match state.v2.writer_paper(principal.user_id(), paper_id).await {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    if paper.kind == persistence::PaperKind::Team && !paper.is_team_leader {
        return error(
            StatusCode::FORBIDDEN,
            "Only the Team Leader can create authoritative Team checkpoints.",
        );
    }
    let exact = match capture_exact_v2_state(&state, &paper).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .create_manual_version(
            paper_id,
            paper.workspace_id,
            exact.document_epoch,
            exact.source_sequence,
            exact.snapshot_id,
            exact.manifest,
            &exact.state_hash,
            principal.user_id(),
            &input.name,
        )
        .await
    {
        Ok(version) => (StatusCode::CREATED, Json(version)).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_versions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
) -> Response {
    let principal = match v2_paper_reader_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.v2.paper_versions(principal.user_id(), paper_id).await {
        Ok(versions) => Json(versions).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, version_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Response {
    let principal = match v2_paper_reader_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .paper_version(principal.user_id(), paper_id, version_id)
        .await
    {
        Ok(version) => Json(version).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_restoration_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
) -> Response {
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .restoration_requests_for_actor(principal.user_id(), Some(paper_id))
        .await
    {
        Ok(requests) => Json(requests).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn v2_create_restoration_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
    Json(input): Json<RestorationRequestInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .create_restoration_request(
            principal.user_id(),
            paper_id,
            input.target_version_id,
            input.reason.as_deref(),
        )
        .await
    {
        Ok(request) => (StatusCode::CREATED, Json(request)).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn v2_assigned_restoration_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .restoration_requests_for_actor(principal.user_id(), None)
        .await
    {
        Ok(requests) => Json(requests).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn v2_leader_reject_restoration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<uuid::Uuid>,
    Json(input): Json<GovernanceDecisionInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .leader_reject_restoration(principal.user_id(), request_id, input.note.as_deref())
        .await
    {
        Ok(request) => Json(request).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn v2_leader_apply_restoration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<uuid::Uuid>,
    Json(input): Json<GovernanceDecisionInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let request = match state
        .v2
        .restoration_requests_for_actor(principal.user_id(), None)
        .await
    {
        Ok(requests) => match requests
            .into_iter()
            .find(|request| request.id == request_id)
        {
            Some(value) => value,
            None => return error(StatusCode::NOT_FOUND, "revert request not found"),
        },
        Err(value) => return v2_error(value),
    };
    if request.state != "REQUESTED" {
        return error(StatusCode::CONFLICT, "revert request is not pending");
    }
    let paper = match state
        .v2
        .writer_paper(principal.user_id(), request.paper_id)
        .await
    {
        Ok(value) if value.kind == persistence::PaperKind::Team && value.is_team_leader => value,
        Ok(_) => {
            return error(
                StatusCode::FORBIDDEN,
                "Only the Team Leader can apply a Team revert.",
            );
        }
        Err(value) => return v2_error(value),
    };
    let exact = match capture_exact_v2_state(&state, &paper).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let safety = ExactRestoreState {
        document_epoch: exact.document_epoch,
        workspace_version: exact.source_sequence,
        snapshot_id: exact.snapshot_id,
        manifest: exact.manifest,
        state_hash: exact.state_hash,
    };
    match state
        .v2
        .apply_team_restoration(
            principal.user_id(),
            request_id,
            safety,
            input.note.as_deref(),
        )
        .await
    {
        Ok(applied) => {
            state
                .collaboration
                .epoch_changed(request.workspace_id, applied.document_epoch)
                .await;
            Json(applied).into_response()
        }
        Err(value) => v2_error(value),
    }
}

async fn v2_restore_personal_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, version_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let paper = match state.v2.writer_paper(principal.user_id(), paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    if paper.kind != persistence::PaperKind::Personal {
        return error(
            StatusCode::FORBIDDEN,
            "Team Paper reverts are controlled by the Team Leader.",
        );
    }
    let exact = match capture_exact_v2_state(&state, &paper).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let safety = ExactRestoreState {
        document_epoch: exact.document_epoch,
        workspace_version: exact.source_sequence,
        snapshot_id: exact.snapshot_id,
        manifest: exact.manifest,
        state_hash: exact.state_hash,
    };
    match state
        .v2
        .apply_personal_restoration(principal.user_id(), paper_id, version_id, safety)
        .await
    {
        Ok(applied) => {
            state
                .collaboration
                .epoch_changed(paper.workspace_id, applied.document_epoch)
                .await;
            Json(applied).into_response()
        }
        Err(value) => v2_error(value),
    }
}

async fn v2_direct_team_revert(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, version_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(input): Json<ConfirmedRevertInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    if !input.confirmed {
        return error(
            StatusCode::BAD_REQUEST,
            "Explicit confirmation is required to revert Team history.",
        );
    }
    let principal = match writer_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let paper = match state.v2.writer_paper(principal.user_id(), paper_id).await {
        Ok(value) if value.kind == persistence::PaperKind::Team && value.is_team_leader => value,
        Ok(_) => {
            return error(
                StatusCode::FORBIDDEN,
                "Only the Team Leader can revert Team history.",
            );
        }
        Err(value) => return v2_error(value),
    };
    let exact = match capture_exact_v2_state(&state, &paper).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let safety = ExactRestoreState {
        document_epoch: exact.document_epoch,
        workspace_version: exact.source_sequence,
        snapshot_id: exact.snapshot_id,
        manifest: exact.manifest,
        state_hash: exact.state_hash,
    };
    match state
        .v2
        .apply_direct_team_restoration(principal.user_id(), paper_id, version_id, safety)
        .await
    {
        Ok(applied) => {
            state
                .collaboration
                .epoch_changed(paper.workspace_id, applied.document_epoch)
                .await;
            Json(applied).into_response()
        }
        Err(value) => v2_error(value),
    }
}

async fn v2_compare_versions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
    Query(query): Query<V2CompareQuery>,
) -> Response {
    let principal = match v2_paper_reader_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let from = match state
        .v2
        .paper_version(principal.user_id(), paper_id, query.from)
        .await
    {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    let to = match state
        .v2
        .paper_version(principal.user_id(), paper_id, query.to)
        .await
    {
        Ok(value) => value,
        Err(error_value) => return v2_error(error_value),
    };
    match compare_version_manifests(&state, &from.manifest, &to.manifest).await {
        Ok(value) => Json(value).into_response(),
        Err(response) => response,
    }
}

async fn compare_version_manifests(
    state: &AppState,
    from: &serde_json::Value,
    to: &serde_json::Value,
) -> Result<serde_json::Value, Response> {
    let before: WorkspaceManifestV1 =
        serde_json::from_value(from.get("workspace").cloned().ok_or_else(|| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "version manifest missing workspace",
            )
        })?)
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid version manifest",
            )
        })?;
    let after: WorkspaceManifestV1 =
        serde_json::from_value(to.get("workspace").cloned().ok_or_else(|| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "version manifest missing workspace",
            )
        })?)
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "invalid version manifest",
            )
        })?;
    let added = after
        .files()
        .keys()
        .filter(|path| !before.files().contains_key(*path))
        .map(LogicalPath::as_str)
        .collect::<Vec<_>>();
    let removed = before
        .files()
        .keys()
        .filter(|path| !after.files().contains_key(*path))
        .map(LogicalPath::as_str)
        .collect::<Vec<_>>();
    let changed = before
        .files()
        .iter()
        .filter_map(|(path, old)| {
            after
                .files()
                .get(path)
                .filter(|new| *new != old)
                .map(|_| path)
        })
        .collect::<Vec<_>>();
    let mut diffs = BTreeMap::new();
    for path in &changed {
        let old = before
            .files()
            .get(*path)
            .expect("changed path exists before");
        let new = after.files().get(*path).expect("changed path exists after");
        let old_bytes = state.blobs.get(old.blob_hash).await.map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "version blob unavailable",
            )
        })?;
        let new_bytes = state.blobs.get(new.blob_hash).await.map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "version blob unavailable",
            )
        })?;
        if let (Ok(old_text), Ok(new_text)) = (
            std::str::from_utf8(&old_bytes),
            std::str::from_utf8(&new_bytes),
        ) {
            let mut unified = format!("--- a/{}\n+++ b/{}\n", path.as_str(), path.as_str());
            for line in old_text.lines() {
                unified.push('-');
                unified.push_str(line);
                unified.push('\n');
            }
            for line in new_text.lines() {
                unified.push('+');
                unified.push_str(line);
                unified.push('\n');
            }
            diffs.insert(path.as_str(), unified);
        }
    }
    Ok(serde_json::json!({
        "files_added": added,
        "files_removed": removed,
        "files_changed": changed.iter().map(|path| path.as_str()).collect::<Vec<_>>(),
        "text_diffs": diffs,
    }))
}

async fn v2_submit_build(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
    Json(input): Json<V2BuildInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    if !matches!(input.trigger_type.as_str(), "auto" | "manual") {
        return error(
            StatusCode::BAD_REQUEST,
            "trigger_type must be auto or manual",
        );
    }
    let principal = match v2_paper_reader_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if matches!(principal.kind, PrincipalKind::V2(GlobalRole::Mentor))
        && input.trigger_type != "manual"
    {
        return error(StatusCode::FORBIDDEN, "Mentor builds must be manual");
    }
    let paper = match principal.kind {
        PrincipalKind::V2(GlobalRole::Writer) => {
            match state.v2.writer_paper(principal.user_id(), paper_id).await {
                Ok(value) => value,
                Err(error_value) => return v2_error(error_value),
            }
        }
        PrincipalKind::V2(GlobalRole::Mentor) => {
            match state.v2.review_paper(principal.user_id(), paper_id).await {
                Ok((value, _)) => value,
                Err(error_value) => return v2_error(error_value),
            }
        }
        PrincipalKind::V2(GlobalRole::Admin) | PrincipalKind::Legacy(_) => {
            return error(StatusCode::FORBIDDEN, "Writer or assigned Mentor required");
        }
    };
    let exact = match capture_exact_v2_state(&state, &paper).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let engine = TexEngine::PdfLatex;
    let profile = LatexmkProfileId::parse("safe-v1").expect("static profile is valid");
    let compile_key = match CompileKeyMaterialV1::new(
        exact.snapshot_id,
        engine,
        state.environment.clone(),
        profile.clone(),
        ShellPolicy::Safe,
        true,
    )
    .compile_key()
    {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "compile key failure"),
    };
    let request = V2BuildRequest {
        paper_id,
        workspace_id: paper.workspace_id,
        document_epoch: exact.document_epoch,
        source_sequence: exact.source_sequence,
        snapshot_id: exact.snapshot_id,
        manifest: exact.manifest,
        state_hash: exact.state_hash,
        tenant_id: principal.session.tenant_id,
        user_id: principal.user_id(),
        trigger_type: input.trigger_type,
        compile_key,
        engine,
        tex_environment_id: state.environment.clone(),
        latexmk_profile: profile,
        shell_policy: ShellPolicy::Safe,
        synctex: true,
    };
    match state.v2.submit_v2_build(&request).await {
        Ok(build) => (StatusCode::ACCEPTED, Json(build)).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_build_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<uuid::Uuid>,
) -> Response {
    let principal = match v2_paper_reader_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.v2.v2_build_view(principal.user_id(), paper_id).await {
        Ok(view) => Json(serde_json::json!({
            "build": view,
            "pdf_url": view.current_build_id.map(|_| format!("/api/v2/papers/{paper_id}/artifacts/pdf")),
            "log_url": view.current_build_id.map(|_| format!("/api/v2/papers/{paper_id}/artifacts/log")),
            "synctex_url": view.current_build_id.map(|_| format!("/api/v2/papers/{paper_id}/artifacts/synctex")),
        })).into_response(),
        Err(error_value) => v2_error(error_value),
    }
}

async fn v2_current_artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, kind)): Path<(uuid::Uuid, String)>,
) -> Response {
    let principal = match v2_paper_reader_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .current_v2_artifact(principal.user_id(), paper_id, &kind)
        .await
    {
        Ok(artifact) => match state.blobs.get(artifact.blob_hash).await {
            Ok(bytes) => (
                [
                    (header::CONTENT_TYPE, artifact.content_type),
                    (
                        header::CONTENT_DISPOSITION,
                        format!(
                            "{}; filename=\"{}\"",
                            if kind == "pdf" {
                                "inline"
                            } else {
                                "attachment"
                            },
                            artifact
                                .logical_name
                                .rsplit('/')
                                .next()
                                .unwrap_or("artifact")
                        ),
                    ),
                ],
                bytes,
            )
                .into_response(),
            Err(_) => error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "artifact storage failure",
            ),
        },
        Err(error_value) => v2_error(error_value),
    }
}

fn default_manual_trigger() -> String {
    "manual".to_owned()
}

async fn authorized_file(
    state: &AppState,
    writer: UserId,
    paper_id: uuid::Uuid,
    file_id: uuid::Uuid,
) -> Result<(persistence::WriterPaper, persistence::PaperFile), Response> {
    let paper = state
        .v2
        .writer_paper(writer, paper_id)
        .await
        .map_err(v2_error)?;
    let file = state.v2.paper_file(file_id).await.map_err(v2_error)?;
    if file.workspace_id != paper.workspace_id || file.tombstoned {
        return Err(error(StatusCode::NOT_FOUND, "file not found"));
    }
    let policy = state.v2.file_policy(file_id).await.map_err(v2_error)?;
    if !policy.visible_to_participants() {
        return Err(error(StatusCode::NOT_FOUND, "file not found"));
    }
    Ok((paper, file))
}

fn parse_user_id(value: &str) -> Result<UserId, Response> {
    uuid::Uuid::parse_str(value)
        .map(UserId::from_uuid)
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid user id"))
}

fn parse_user_ids(values: &[String]) -> Result<Vec<UserId>, Response> {
    values.iter().map(|value| parse_user_id(value)).collect()
}

async fn template_seeds(
    state: &AppState,
    template_id: uuid::Uuid,
) -> Result<(LogicalPath, Vec<TemplateSeedFile>, String), Response> {
    let template = state
        .repo
        .template(template_id)
        .await
        .map_err(|_| error(StatusCode::NOT_FOUND, "template not found"))?;
    let records = match state.repo.template_files(template_id).await {
        Ok(value) if !value.is_empty() => value,
        Ok(_) => return Err(error(StatusCode::CONFLICT, "template has no files")),
        Err(_) => {
            return Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "template lookup failed",
            ));
        }
    };
    let main_path = match template
        .main_file
        .as_deref()
        .map(LogicalPath::parse)
        .transpose()
    {
        Ok(Some(value)) => value,
        Ok(None) => return Err(error(StatusCode::CONFLICT, "template has no main file")),
        Err(_) => {
            return Err(error(StatusCode::CONFLICT, "template main file is invalid"));
        }
    };
    let policy = match template.policy_default.as_str() {
        "editable" => V2FilePolicy::Editable,
        "read_only" => V2FilePolicy::ContentReadOnly,
        "managed" => V2FilePolicy::TemplateManaged,
        _ => {
            return Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "template policy is invalid",
            ));
        }
    };
    let mut identity_material = format!(
        "{template_id}:{}:{}",
        template.main_file.as_deref().unwrap_or(""),
        template.policy_default
    );
    let mut seeds = Vec::with_capacity(records.len());
    for record in records {
        let path = LogicalPath::parse(&record.path)
            .map_err(|_| error(StatusCode::CONFLICT, "template contains an invalid path"))?;
        let _ = write!(
            identity_material,
            "|{}:{}:{}",
            path.as_str(),
            record.blob_hash,
            record.size_bytes
        );
        seeds.push(TemplateSeedFile {
            path,
            blob_hash: record.blob_hash,
            size_bytes: record.size_bytes,
            policy,
        });
    }
    Ok((main_path, seeds, digest(&identity_material)))
}

fn csv_escape(value: &str) -> String {
    let neutralized;
    let value = if value.starts_with(['=', '+', '-', '@']) {
        neutralized = format!("'{value}");
        neutralized.as_str()
    } else {
        value
    };
    if value.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

async fn institution_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedPrincipal, Response> {
    let principal = admin_session(state, headers).await?;
    if !matches!(principal.kind, PrincipalKind::V2(GlobalRole::Admin)) {
        return Err(error(
            StatusCode::FORBIDDEN,
            "V2 Admin required for institution administration",
        ));
    }
    Ok(principal)
}

fn institution_error(error_value: InstitutionError) -> Response {
    match error_value {
        InstitutionError::InvalidInput(message) => error(StatusCode::BAD_REQUEST, message),
        InstitutionError::NotFound => error(StatusCode::NOT_FOUND, "institution record not found"),
        InstitutionError::Conflict(message) => error(StatusCode::CONFLICT, message),
        InstitutionError::Database(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "institution persistence failure",
        ),
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "owned V2 errors compose directly with Result::map_err at HTTP boundaries"
)]
fn v2_error(error_value: V2Error) -> Response {
    match error_value {
        V2Error::RoleMissing { .. } | V2Error::RoleForbidden { .. } => {
            error(StatusCode::FORBIDDEN, error_value.to_string())
        }
        V2Error::PersonalPaperOwnershipConflict { .. }
        | V2Error::TeamMembershipConflict { .. }
        | V2Error::WorkspaceConflict { .. }
        | V2Error::DuplicateFilePath { .. }
        | V2Error::Conflict { .. }
        | V2Error::VersionConflict { .. } => error(StatusCode::CONFLICT, error_value.to_string()),
        V2Error::NotFound { .. } => error(StatusCode::NOT_FOUND, error_value.to_string()),
        V2Error::InvalidRole { .. }
        | V2Error::InvalidStatus { .. }
        | V2Error::InvalidPath { .. }
        | V2Error::InvalidName => error(StatusCode::BAD_REQUEST, error_value.to_string()),
        V2Error::Integrity { .. } | V2Error::Database(_) => {
            error(StatusCode::INTERNAL_SERVER_ERROR, "V2 persistence failure")
        }
    }
}

async fn admin_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedPrincipal, Response> {
    let principal = principal_auth(state, headers).await?;
    if !matches!(
        principal.kind,
        PrincipalKind::V2(GlobalRole::Admin) | PrincipalKind::Legacy(AccountType::Admin)
    ) {
        return Err(error(
            StatusCode::FORBIDDEN,
            "administrator capability required",
        ));
    }
    Ok(principal)
}
async fn admin_overview(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let session = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.repo.admin_overview().await {
        Ok(mut overview) => {
            overview["version"] = serde_json::Value::String("latex-core 0.1.0".into());
            overview["current_admin"] = serde_json::Value::String(session.email().to_owned());
            Json(overview).into_response()
        }
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn admin_users(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    match state.repo.admin_users().await { Ok(users) => Json(users.into_iter().map(|user| serde_json::json!({"id":user.user_id.to_string(),"email":user.email,"account_type":user.account_type,"enabled":user.enabled,"created_at":user.created_at})).collect::<Vec<_>>()).into_response(), Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure") }
}
async fn admin_create_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AdminUserInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    let email = match auth::normalized_email(&input.email) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid email"),
    };
    if !matches!(
        input.account_type.as_str(),
        "student" | "professor" | "admin"
    ) {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid institutional account type",
        );
    }
    let generated = input.password.is_none();
    let password = input.password.unwrap_or_else(auth::temporary_password);
    let hash = match if generated {
        auth::hash_temporary_password(&password)
    } else {
        auth::hash_password(&password)
    } {
        Ok(value) => value,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "password must be 12-256 characters",
            );
        }
    };
    match state.repo.create_account(&email, &hash).await {
        Ok(_) => match state
            .repo
            .set_user_account_type(&email, &input.account_type)
            .await
        {
            Ok(()) => Json(serde_json::json!({"email":email,"temporary_password":password}))
                .into_response(),
            Err(_) => error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "account type update failed",
            ),
        },
        Err(AppError::Conflict) => error(StatusCode::CONFLICT, "account exists"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "account creation failed"),
    }
}
async fn admin_patch_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(email): Path<String>,
    Json(input): Json<AdminUserPatch>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    let email = match auth::normalized_email(&email) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid email"),
    };
    if let Some(account_type) = input.account_type {
        match state
            .repo
            .set_user_account_type(&email, &account_type)
            .await
        {
            Ok(()) => {}
            Err(AppError::Integrity { .. }) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "invalid institutional account type",
                );
            }
            Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
        }
    }
    if let Some(enabled) = input.enabled {
        match state.repo.set_user_enabled(&email, enabled).await {
            Ok(()) => {}
            Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
        }
    }
    StatusCode::NO_CONTENT.into_response()
}
async fn admin_reset_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(email): Path<String>,
    Json(input): Json<AdminPasswordInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    let email = match auth::normalized_email(&email) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid email"),
    };
    let generated = input.password.is_none();
    let password = input.password.unwrap_or_else(auth::temporary_password);
    let hash = match if generated {
        auth::hash_temporary_password(&password)
    } else {
        auth::hash_password(&password)
    } {
        Ok(value) => value,
        Err(_) => {
            return error(
                StatusCode::BAD_REQUEST,
                "password must be 12-256 characters",
            );
        }
    };
    match state.repo.reset_password(&email, &hash).await {
        Ok(()) => {
            Json(serde_json::json!({"email":email,"temporary_password":password})).into_response()
        }
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn admin_delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(email): Path<String>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    match state.repo.delete_user_if_unreferenced(&email).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        // Account deletion is deliberately conservative: this endpoint never
        // attempts a cascade, so a persistence failure is reported as an
        // unavailable safe deletion rather than exposing database internals.
        Err(AppError::Integrity { .. } | AppError::Database(_)) => error(
            StatusCode::CONFLICT,
            "User owns resources and cannot be deleted. Disable the account instead.",
        ),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn admin_data(
    State(state): State<AppState>,
    headers: HeaderMap,
    kind: &'static str,
) -> Response {
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    let value = match kind {
        "teams" => state.repo.admin_teams().await,
        "research_groups" => state.repo.admin_research_groups().await,
        "projects" => state.repo.admin_projects().await,
        "jobs" => state.repo.admin_jobs().await,
        "audit" => state.repo.admin_audit().await,
        _ => Ok(Vec::new()),
    };
    match value {
        Ok(value) => Json(value).into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn admin_teams(State(state): State<AppState>, headers: HeaderMap) -> Response {
    admin_data(State(state), headers, "teams").await
}
async fn admin_research_groups(State(state): State<AppState>, headers: HeaderMap) -> Response {
    admin_data(State(state), headers, "research_groups").await
}
async fn admin_projects(State(state): State<AppState>, headers: HeaderMap) -> Response {
    admin_data(State(state), headers, "projects").await
}
async fn admin_jobs(State(state): State<AppState>, headers: HeaderMap) -> Response {
    admin_data(State(state), headers, "jobs").await
}
async fn admin_audit(State(state): State<AppState>, headers: HeaderMap) -> Response {
    admin_data(State(state), headers, "audit").await
}
async fn admin_templates(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    let templates = match state.repo.list_templates().await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    let mut response = Vec::with_capacity(templates.len());
    for template in templates {
        let files = match state.repo.template_files(template.id).await {
            Ok(value) => value,
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
        };
        let usage_count = match state.repo.template_paper_team_usage(template.id).await {
            Ok(value) => value,
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
        };
        response.push(serde_json::json!({
            "id":template.id.to_string(),"name":template.name,"description":template.description,
            "main_file":template.main_file,"policy_default":template.policy_default,
            "created_at":template.created_at,"usage_count":usage_count,"pinned":usage_count > 0,
            "tex_files":files.iter().filter(|file| std::path::Path::new(&file.path).extension().is_some_and(|extension| extension.eq_ignore_ascii_case("tex"))).map(|file| &file.path).collect::<Vec<_>>(),
            "update_status":"Template source is immutable; metadata and Main selection may be edited"
        }));
    }
    Json(response).into_response()
}
async fn admin_v2_template_preview(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    let body = match template_archive_field(multipart).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let imported = match archive::read_archive(body.as_ref()) {
        Ok(value) => value,
        Err(error_value) => return import_error(error_value),
    };
    if !imported
        .files
        .iter()
        .any(|file| file.path.extension() == Some("tex"))
    {
        return error(
            StatusCode::BAD_REQUEST,
            "template archive contains no TeX file",
        );
    }
    Json(serde_json::json!({
        "schema_version": 1,
        "detected_main": imported.detected_main.as_ref().map(LogicalPath::as_str),
        "files": imported.files.iter().map(|file| serde_json::json!({
            "path": file.path.as_str(),
            "size_bytes": file.bytes.len(),
            "is_tex": file.path.extension() == Some("tex"),
        })).collect::<Vec<_>>(),
    }))
    .into_response()
}

#[allow(
    clippy::too_many_lines,
    reason = "archive validation, blob persistence, and template metadata remain together at the import boundary"
)]
async fn admin_v2_template_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Multipart,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !matches!(principal.kind, PrincipalKind::V2(GlobalRole::Admin)) {
        return error(
            StatusCode::FORBIDDEN,
            "V2 Admin required for template import",
        );
    }
    let upload = match template_import_fields(multipart).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let name = upload.name.trim();
    if name.is_empty() || name.len() > 200 {
        return error(StatusCode::BAD_REQUEST, "invalid template name");
    }
    let description = upload
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if description.is_some_and(|value| value.len() > 2_000) {
        return error(StatusCode::BAD_REQUEST, "template description is too long");
    }
    let imported = match archive::read_archive(upload.archive.as_ref()) {
        Ok(value) => value,
        Err(error_value) => return import_error(error_value),
    };
    let main = match upload.main.as_deref().filter(|value| !value.is_empty()) {
        Some(value) => match LogicalPath::parse(value) {
            Ok(path) => path,
            Err(_) => return error(StatusCode::BAD_REQUEST, "invalid template main path"),
        },
        None => {
            if let Some(path) = imported.detected_main.clone() {
                path
            } else {
                if !imported
                    .files
                    .iter()
                    .any(|file| file.path.extension() == Some("tex"))
                {
                    return error(
                        StatusCode::BAD_REQUEST,
                        "template archive contains no TeX file",
                    );
                }
                return error(
                    StatusCode::CONFLICT,
                    "select a main TeX file for this template",
                );
            }
        }
    };
    if main.extension() != Some("tex") || !imported.files.iter().any(|file| file.path == main) {
        return error(
            StatusCode::BAD_REQUEST,
            "template main must name an archived TeX file",
        );
    }
    let mut records = Vec::with_capacity(imported.files.len());
    for file in imported.files {
        let stored = match state.blobs.put(file.bytes).await {
            Ok(value) => value,
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "blob storage failure"),
        };
        records.push(AppTemplateFileRecord {
            path: file.path.to_string(),
            blob_hash: stored.hash(),
            size_bytes: stored.size_bytes(),
        });
    }
    let id = uuid::Uuid::new_v4();
    match state
        .repo
        .create_template(id, name, description, Some(main.as_str()), &records)
        .await
    {
        Ok(()) => {
            let mut identity_material = format!("{id}:{}", main.as_str());
            for file in &records {
                let _ = write!(
                    identity_material,
                    "|{}:{}:{}",
                    file.path, file.blob_hash, file.size_bytes
                );
            }
            tracing::info!(template_id=%id, admin_user_id=%principal.user_id(), file_count=records.len(), "V2 template imported");
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "schema_version": 1,
                    "id": id,
                    "name": name,
                    "description": description,
                    "main_file": main.as_str(),
                    "source_identity": digest(&identity_material),
                    "files": records.iter().map(|file| serde_json::json!({
                        "path": file.path,
                        "blob_hash": file.blob_hash,
                        "size_bytes": file.size_bytes,
                    })).collect::<Vec<_>>(),
                })),
            )
                .into_response()
        }
        Err(AppError::Conflict) => error(StatusCode::CONFLICT, "template name already exists"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "template import failed"),
    }
}

#[derive(Default)]
struct TemplateImportFields {
    name: String,
    description: Option<String>,
    main: Option<String>,
    archive: Bytes,
}

async fn template_archive_field(mut multipart: Multipart) -> Result<Bytes, Response> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid template upload"))?
    {
        if field.name() == Some("archive") {
            return field
                .bytes()
                .await
                .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid template upload"));
        }
    }
    Err(error(StatusCode::BAD_REQUEST, "Template ZIP is required"))
}

async fn template_import_fields(
    mut multipart: Multipart,
) -> Result<TemplateImportFields, Response> {
    let mut upload = TemplateImportFields::default();
    let mut has_archive = false;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid template upload"))?
    {
        match field.name() {
            Some("name") => {
                upload.name = field
                    .text()
                    .await
                    .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid template name"))?;
            }
            Some("description") => {
                upload.description =
                    Some(field.text().await.map_err(|_| {
                        error(StatusCode::BAD_REQUEST, "invalid template description")
                    })?);
            }
            Some("main") => {
                upload.main = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid template main"))?,
                );
            }
            Some("archive") if !has_archive => {
                upload.archive = field
                    .bytes()
                    .await
                    .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid template upload"))?;
                has_archive = true;
            }
            Some("archive") => {
                return Err(error(
                    StatusCode::BAD_REQUEST,
                    "only one Template ZIP may be uploaded",
                ));
            }
            _ => {}
        }
    }
    if !has_archive {
        return Err(error(StatusCode::BAD_REQUEST, "Template ZIP is required"));
    }
    Ok(upload)
}

async fn admin_v2_template_edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
    Json(input): Json<V2TemplateEditInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    let name = input.name.trim();
    let description = input
        .description
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let main = match LogicalPath::parse(&input.main_file) {
        Ok(value) if value.extension() == Some("tex") => value,
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "Main must be an existing .tex file",
            );
        }
    };
    if name.is_empty() || name.len() > 200 || description.is_some_and(|value| value.len() > 2_000) {
        return error(StatusCode::BAD_REQUEST, "invalid template metadata");
    }
    match state
        .repo
        .update_template_metadata(id, name, description, main.as_str())
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(AppError::NotFound) => error(
            StatusCode::BAD_REQUEST,
            "Main must be an existing .tex file",
        ),
        Err(AppError::Conflict) => error(StatusCode::CONFLICT, "template name already exists"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "template update failed"),
    }
}

async fn admin_v2_template_remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<uuid::Uuid>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    match state.repo.delete_template_if_unused(id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(AppError::Conflict) => error(
            StatusCode::CONFLICT,
            "Template is in use by a Paper Team and cannot be removed.",
        ),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "template not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "template removal failed"),
    }
}
async fn admin_system(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = admin_session(&state, &headers).await {
        return response;
    }
    Json(serde_json::json!({"version":"latex-core 0.1.0","database":"application database configured","queue":"durable PostgreSQL queue","compiler":"M7 verified by operator doctor","host_operations":"Operator service required"})).into_response()
}
async fn projects(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    match state.repo.list_projects(s.user_id).await {
        Ok(records) => Json(
            records
                .into_iter()
                .map(|p| ProjectListWire {
                    id: p.workspace_id.to_string(),
                    name: p.name,
                    created_at: p.created_at,
                    updated_at: p.updated_at,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<NewProject>,
) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    };
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let name = input.name.trim();
    if name.is_empty() || name.len() > 200 {
        return error(StatusCode::BAD_REQUEST, "invalid project name");
    };
    let id = WorkspaceId::new();
    let main = match LogicalPath::parse("main.tex") {
        Ok(v) => v,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "path failure"),
    };
    if state.workspaces.create_workspace(s.tenant_id,s.user_id,id,main,Bytes::from_static(b"\\documentclass{article}\n\\begin{document}\nHello, LaTeX Core!\n\\end{document}\n")).await.is_err(){return error(StatusCode::INTERNAL_SERVER_ERROR,"workspace creation failed")};
    match state.repo.create_project(id, s.user_id, name).await {
        Ok(()) => project_response(&state, s.user_id, id).await,
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "project creation failed"),
    }
}
fn research_group_wire(group: persistence::ResearchGroupRecord) -> ResearchGroupWire {
    ResearchGroupWire {
        id: group.id.to_string(),
        name: group.name,
        workspace_id: group.workspace_id.to_string(),
        owner_user_id: group.owner_user_id.to_string(),
        created_at: group.created_at,
        updated_at: group.updated_at,
    }
}
async fn research_groups(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.repo.research_groups_for_user(session.user_id).await {
        Ok(groups) => Json(
            groups
                .into_iter()
                .map(research_group_wire)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn create_research_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<NewResearchGroup>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let name = match project_name(&input.name) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let workspace = WorkspaceId::new();
    let main = match LogicalPath::parse("main.tex") {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "path failure"),
    };
    if state.workspaces.create_workspace(session.tenant_id, session.user_id, workspace, main, Bytes::from_static(b"\\documentclass{article}\n\\begin{document}\nResearch group project\n\\end{document}\n")).await.is_err() { return error(StatusCode::INTERNAL_SERVER_ERROR, "workspace creation failed"); }
    match state
        .repo
        .create_research_group(session.user_id, workspace, name)
        .await
    {
        Ok(group) => (StatusCode::CREATED, Json(research_group_wire(group))).into_response(),
        Err(AppError::Conflict) => error(StatusCode::CONFLICT, "research group already exists"),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "research group creation failed",
        ),
    }
}
async fn research_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    match state
        .repo
        .research_group_for_user(session.user_id, id)
        .await
    {
        Ok(group) => Json(research_group_wire(group)).into_response(),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn research_group_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    match state.repo.research_group_members(session.user_id, id).await {
        Ok(members) => Json(
            members
                .into_iter()
                .map(|member| ResearchGroupMemberWire {
                    user_id: member.user_id.to_string(),
                    email: member.email,
                    joined_at: member.joined_at,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn add_research_group_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<ResearchGroupMemberInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let email = match auth::normalized_email(&input.email) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid email"),
    };
    let target = match state.repo.user_by_email(&email).await {
        Ok(Some(user)) if user.enabled => user.user_id,
        Ok(Some(_)) => return error(StatusCode::BAD_REQUEST, "user is disabled"),
        Ok(None) => return error(StatusCode::NOT_FOUND, "user not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    match state
        .repo
        .add_research_group_member(session.user_id, id, target)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(AppError::Forbidden) => error(
            StatusCode::FORBIDDEN,
            "research group owner capability required",
        ),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn remove_research_group_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, user)): Path<(String, String)>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let target = match parsed::<UserId>(&user) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .repo
        .remove_research_group_member(session.user_id, id, target)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(AppError::Forbidden) => error(
            StatusCode::FORBIDDEN,
            "research group owner capability required",
        ),
        Err(AppError::Integrity { .. }) => error(
            StatusCode::CONFLICT,
            "the research group owner cannot be removed",
        ),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn rename_research_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<RenameResearchGroup>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let name = match project_name(&input.name) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .repo
        .rename_research_group(session.user_id, id, name)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(AppError::Forbidden) => error(
            StatusCode::FORBIDDEN,
            "research group owner capability required",
        ),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn delete_research_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    match state.repo.delete_research_group(session.user_id, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(AppError::Forbidden) => error(
            StatusCode::FORBIDDEN,
            "research group owner capability required",
        ),
        Err(_) => error(
            StatusCode::CONFLICT,
            "Research group deletion is unavailable because its workspace is retained.",
        ),
    }
}
async fn teams(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.repo.teams_for_user(session.user_id).await {
        Ok(records) => Json(
            records
                .into_iter()
                .map(|team| TeamWire {
                    id: team.id.to_string(),
                    name: team.name,
                    group_type: team.group_type.as_str().to_owned(),
                    created_at: team.created_at,
                    updated_at: team.updated_at,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn create_team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<NewTeam>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let name = match project_name(&input.name) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let group_type = match GroupType::parse(input.group_type.as_deref().unwrap_or("research_team"))
    {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid group type"),
    };
    match state
        .repo
        .create_group(session.user_id, name, group_type)
        .await
    {
        Ok(team) => (
            StatusCode::CREATED,
            Json(TeamWire {
                id: team.id.to_string(),
                name: team.name,
                group_type: team.group_type.as_str().to_owned(),
                created_at: team.created_at,
                updated_at: team.updated_at,
            }),
        )
            .into_response(),
        Err(AppError::Conflict) => error(StatusCode::CONFLICT, "team already exists"),
        Err(AppError::Forbidden) => error(
            StatusCode::FORBIDDEN,
            "professor or administrator capability required",
        ),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn team(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    match state.repo.team(session.user_id, id).await {
        Ok(team) => Json(TeamWire {
            id: team.id.to_string(),
            name: team.name,
            group_type: team.group_type.as_str().to_owned(),
            created_at: team.created_at,
            updated_at: team.updated_at,
        })
        .into_response(),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn team_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    match state.repo.team_members(session.user_id, id).await {
        Ok(records) => Json(
            records
                .into_iter()
                .map(|member| TeamMemberWire {
                    user_id: member.user_id.to_string(),
                    email: member.email,
                    group_manager: member.group_manager,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn set_team_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<TeamMemberInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let email = match auth::normalized_email(&input.email) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid email"),
    };
    let target = match state.repo.user_by_email(&email).await {
        Ok(Some(user)) => user.user_id,
        Ok(None) => return error(StatusCode::NOT_FOUND, "user not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    match state
        .repo
        .set_group_member(session.user_id, id, target, input.group_manager)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(AppError::Forbidden) => {
            error(StatusCode::FORBIDDEN, "team manager capability required")
        }
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn remove_team_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, user)): Path<(String, String)>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let team_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let target = match parsed::<UserId>(&user) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let unpublished_change_count = match state
        .repo
        .unpublished_change_count_for_member(session.user_id, team_id, target)
        .await
    {
        Ok(value) => value,
        Err(AppError::Forbidden) => {
            return error(StatusCode::FORBIDDEN, "team manager capability required");
        }
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    match state
        .repo
        .remove_team_member(session.user_id, team_id, target)
        .await
    {
        Ok(()) => Json(MemberRemovalWire {
            unpublished_change_count,
        })
        .into_response(),
        Err(AppError::Forbidden) => {
            error(StatusCode::FORBIDDEN, "team manager capability required")
        }
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn team_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let team_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    match state.repo.team_projects(session.user_id, team_id).await {
        Ok(records) => Json(
            records
                .into_iter()
                .map(|project| TeamProjectWire {
                    id: project.id.to_string(),
                    workspace_id: project.workspace_id.to_string(),
                    name: project.name,
                    canonical_generation: project.canonical_generation,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn create_team_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<TeamProjectInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let team_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let name = match project_name(&input.name) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let members = match state.repo.team_members(session.user_id, team_id).await {
        Ok(value) => value,
        Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    if session.account_type != "admin"
        && !members
            .iter()
            .any(|member| member.user_id == session.user_id && member.group_manager)
    {
        return error(StatusCode::FORBIDDEN, "team manager capability required");
    }
    let workspace = WorkspaceId::new();
    let main = match LogicalPath::parse("main.tex") {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "path failure"),
    };
    if state.workspaces.create_workspace(session.tenant_id, session.user_id, workspace, main, Bytes::from_static(b"\\documentclass{article}\n\\begin{document}\nHello, team LaTeX Core!\n\\end{document}\n")).await.is_err() { return error(StatusCode::INTERNAL_SERVER_ERROR, "workspace creation failed"); }
    let files = match state.workspaces.restore(workspace).await {
        Ok(value) => value
            .files()
            .iter()
            .map(|(path, file)| TeamFileRecord {
                path: path.as_str().to_owned(),
                blob_hash: file.blob_hash(),
                size_bytes: file.size_bytes(),
                revision: 1,
                policy: FilePolicy::Editable,
            })
            .collect::<Vec<_>>(),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"),
    };
    match state
        .repo
        .create_team_project(session.user_id, team_id, workspace, name, &files)
        .await
    {
        Ok(project) => (
            StatusCode::CREATED,
            Json(TeamProjectWire {
                id: project.id.to_string(),
                workspace_id: project.workspace_id.to_string(),
                name: project.name,
                canonical_generation: project.canonical_generation,
            }),
        )
            .into_response(),
        Err(AppError::Forbidden) => {
            error(StatusCode::FORBIDDEN, "team manager capability required")
        }
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
#[allow(
    clippy::too_many_lines,
    reason = "template blob validation and workspace creation must remain adjacent to the authorization boundary"
)]
async fn create_team_project_from_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((team, template)): Path<(String, String)>,
    Json(input): Json<TeamTemplateProjectInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let team_id = match uuid::Uuid::parse_str(&team) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let template_id = match uuid::Uuid::parse_str(&template) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let name = match project_name(&input.name) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(template_error) = state
        .repo
        .assert_template_visible(session.user_id, template_id)
        .await
    {
        return match template_error {
            AppError::Forbidden => error(StatusCode::FORBIDDEN, "template use capability required"),
            _ => error(StatusCode::NOT_FOUND, "not found"),
        };
    }
    let template = match state.repo.template(template_id).await {
        Ok(value) => value,
        Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    let main = match template.main_file {
        Some(value) => match LogicalPath::parse(&value) {
            Ok(value) => value,
            Err(_) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "template data is invalid",
                );
            }
        },
        None => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "template data is invalid",
            );
        }
    };
    let records = match state.repo.template_files(template_id).await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    let mut files = Vec::with_capacity(records.len());
    for record in records {
        let path = match LogicalPath::parse(&record.path) {
            Ok(value) => value,
            Err(_) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "template data is invalid",
                );
            }
        };
        let bytes = match state.blobs.get(record.blob_hash).await {
            Ok(value) if u64::try_from(value.len()).ok() == Some(record.size_bytes) => value,
            Ok(_) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "template data is invalid",
                );
            }
            Err(_) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "template storage failure",
                );
            }
        };
        files.push(archive::ImportedFile { path, bytes });
    }
    if !files.iter().any(|file| file.path == main) {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "template data is invalid",
        );
    }
    let workspace = WorkspaceId::new();
    if state
        .workspaces
        .create_workspace_from_files(
            session.tenant_id,
            session.user_id,
            workspace,
            files
                .into_iter()
                .map(|file| (file.path, file.bytes))
                .collect(),
            Some(main),
        )
        .await
        .is_err()
    {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "workspace creation failed",
        );
    }
    match state
        .repo
        .instantiate_team_project_from_template(
            session.user_id,
            team_id,
            workspace,
            name,
            template_id,
        )
        .await
    {
        Ok(project) => (
            StatusCode::CREATED,
            Json(TeamProjectWire {
                id: project.id.to_string(),
                workspace_id: project.workspace_id.to_string(),
                name: project.name,
                canonical_generation: project.canonical_generation,
            }),
        )
            .into_response(),
        Err(AppError::Forbidden) => error(
            StatusCode::FORBIDDEN,
            "not authorized to create this team project",
        ),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "project creation failed"),
    }
}
async fn publish_draft(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let project_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let path = match LogicalPath::parse(&path) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid path"),
    };
    let draft = match state
        .repo
        .draft_for_user(session.user_id, project_id, path.as_str())
        .await
    {
        Ok(Some(value)) => value,
        Ok(None) => return error(StatusCode::NOT_FOUND, "draft not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    match state.blobs.get(draft.blob_hash).await {
        Ok(bytes) if u64::try_from(bytes.len()).ok() == Some(draft.size_bytes) => {}
        _ => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "draft blob is unavailable",
            );
        }
    }
    match state
        .repo
        .publish_draft(session.user_id, project_id, path.as_str())
        .await
    {
        Ok(PublishResult::Published {
            canonical_generation,
            workspace_version,
            file_revision,
        }) => Json(PublishWire {
            published: true,
            canonical_generation: Some(canonical_generation),
            workspace_version: Some(workspace_version),
            file_revision,
        })
        .into_response(),
        Ok(PublishResult::Conflict {
            current_file_revision,
        }) => (
            StatusCode::CONFLICT,
            Json(PublishWire {
                published: false,
                canonical_generation: None,
                workspace_version: None,
                file_revision: current_file_revision,
            }),
        )
            .into_response(),
        Err(AppError::Forbidden) => error(StatusCode::FORBIDDEN, "protected by project policy"),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "draft not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn publish_change_set(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let project_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    match state
        .repo
        .publish_change_set(session.user_id, project_id)
        .await
    {
        Ok(ChangeSetPublishResult::Published {
            canonical_generation,
            workspace_version,
            change_count,
        }) => Json(ChangeSetPublishWire {
            published: true,
            canonical_generation: Some(canonical_generation),
            workspace_version: Some(workspace_version),
            change_count: Some(change_count),
            conflicts: Vec::new(),
        })
        .into_response(),
        Ok(ChangeSetPublishResult::Conflict { conflicts }) => (
            StatusCode::CONFLICT,
            Json(ChangeSetPublishWire {
                published: false,
                canonical_generation: None,
                workspace_version: None,
                change_count: None,
                conflicts: conflicts
                    .into_iter()
                    .map(|conflict| ChangeConflictWire {
                        path: conflict.path,
                        destination: conflict.destination,
                        reason: conflict.reason.as_str(),
                    })
                    .collect(),
            }),
        )
            .into_response(),
        Err(AppError::Forbidden) => error(StatusCode::FORBIDDEN, "protected by project policy"),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "no unpublished changes"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn project_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let project_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    match state
        .repo
        .project_members(session.user_id, project_id)
        .await
    {
        Ok(records) => Json(
            records
                .into_iter()
                .map(|member| ProjectMemberWire {
                    user_id: member.user_id.to_string(),
                    email: member.email,
                    writer: member.writer,
                    mentor: member.mentor,
                    project_manager: member.project_manager,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(AppError::Forbidden) => {
            error(StatusCode::FORBIDDEN, "project manager capability required")
        }
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn set_project_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<ProjectMemberInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let project_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let email = match auth::normalized_email(&input.email) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid email"),
    };
    let target = match state.repo.user_by_email(&email).await {
        Ok(Some(user)) => user.user_id,
        Ok(None) => return error(StatusCode::NOT_FOUND, "user not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    match state
        .repo
        .set_project_member(
            session.user_id,
            project_id,
            target,
            ProjectRoles {
                writer: input.writer,
                mentor: input.mentor,
                project_manager: input.project_manager,
            },
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(AppError::Forbidden) => error(
            StatusCode::FORBIDDEN,
            "you do not have permission to manage project roles",
        ),
        Err(AppError::Integrity { message })
            if message == "a project must retain at least one project manager" =>
        {
            error(
                StatusCode::CONFLICT,
                "you must keep at least one Project Manager",
            )
        }
        Err(AppError::Integrity { message })
            if message == "a project member must have at least one project role" =>
        {
            error(StatusCode::BAD_REQUEST, "select at least one project role")
        }
        Err(AppError::Integrity { message })
            if message == "a project member must belong to the team" =>
        {
            error(
                StatusCode::BAD_REQUEST,
                "the user must belong to the team before project access can be granted",
            )
        }
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn remove_project_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, user)): Path<(String, String)>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let project_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let target = match parsed::<UserId>(&user) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .repo
        .remove_project_member(session.user_id, project_id, target)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(AppError::Forbidden) => error(
            StatusCode::FORBIDDEN,
            "you do not have permission to manage project roles",
        ),
        Err(AppError::Integrity { message })
            if message == "a project must retain at least one project manager" =>
        {
            error(
                StatusCode::CONFLICT,
                "you must keep at least one Project Manager",
            )
        }
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn set_file_policy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, path)): Path<(String, String)>,
    Json(input): Json<FilePolicyInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let project_id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    let path = match LogicalPath::parse(&path) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid path"),
    };
    let policy = match FilePolicy::parse(&input.policy) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid policy"),
    };
    match state
        .repo
        .set_file_policy(session.user_id, project_id, path.as_str(), policy)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(AppError::Forbidden) => {
            error(StatusCode::FORBIDDEN, "team manager capability required")
        }
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn import_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ImportQuery>,
    body: Bytes,
) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let name = match project_name(&query.name) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let archive = match archive::read_archive(body.as_ref()) {
        Ok(value) => value,
        Err(error) => return import_error(error),
    };
    create_imported_project(&state, session.tenant_id, session.user_id, name, archive).await
}
async fn templates(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.repo.list_templates_for_user(session.user_id).await {
        Ok(records) => Json(
            records
                .into_iter()
                .map(|template| TemplateWire {
                    id: template.id.to_string(),
                    name: template.name,
                    description: template.description,
                    main_file: template.main_file,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn create_project_from_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<NewProject>,
) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    }
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let name = match project_name(&input.name) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let id = match uuid::Uuid::parse_str(&id) {
        Ok(value) => value,
        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
    };
    if state
        .repo
        .assert_template_visible(session.user_id, id)
        .await
        .is_err()
    {
        return error(StatusCode::NOT_FOUND, "not found");
    }
    let template = match state.repo.template(id).await {
        Ok(value) => value,
        Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    let records = match state.repo.template_files(id).await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    let mut files = Vec::with_capacity(records.len());
    for record in records {
        let path = match LogicalPath::parse(&record.path) {
            Ok(value) => value,
            Err(_) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "template data is invalid",
                );
            }
        };
        let bytes = match state.blobs.get(record.blob_hash).await {
            Ok(value) => value,
            Err(_) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "template storage failure",
                );
            }
        };
        if u64::try_from(bytes.len()).ok() != Some(record.size_bytes) {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "template data is invalid",
            );
        }
        files.push(archive::ImportedFile { path, bytes });
    }
    let main = match template.main_file {
        Some(value) => match LogicalPath::parse(&value) {
            Ok(value) => Some(value),
            Err(_) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "template data is invalid",
                );
            }
        },
        None => None,
    };
    create_imported_project(
        &state,
        session.tenant_id,
        session.user_id,
        name,
        archive::ImportedArchive {
            files,
            detected_main: main,
        },
    )
    .await
}
async fn create_imported_project(
    state: &AppState,
    tenant: core_types::TenantId,
    owner: UserId,
    name: &str,
    archive: archive::ImportedArchive,
) -> Response {
    let id = WorkspaceId::new();
    let main = archive.detected_main;
    let files = archive
        .files
        .into_iter()
        .map(|file| (file.path, file.bytes))
        .collect();
    if state
        .workspaces
        .create_workspace_from_files(tenant, owner, id, files, main)
        .await
        .is_err()
    {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "workspace import failed");
    }
    match state.repo.create_project(id, owner, name).await {
        Ok(()) => project_response(state, owner, id).await,
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "project import failed"),
    }
}
fn project_name(value: &str) -> Result<&str, Response> {
    let name = value.trim();
    if name.is_empty() || name.len() > 200 {
        Err(error(StatusCode::BAD_REQUEST, "invalid project name"))
    } else {
        Ok(name)
    }
}
fn import_error(archive_error: archive::ArchiveError) -> Response {
    let (status, message) = match archive_error {
        archive::ArchiveError::ArchiveTooLarge => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "archive exceeds upload size limit",
        ),
        archive::ArchiveError::ExpandedTooLarge => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "archive exceeds project size limit",
        ),
        archive::ArchiveError::TooManyFiles => {
            (StatusCode::BAD_REQUEST, "archive contains too many files")
        }
        archive::ArchiveError::UnsafePath => {
            (StatusCode::BAD_REQUEST, "archive contains an unsafe path")
        }
        archive::ArchiveError::UnsupportedEntry => (
            StatusCode::BAD_REQUEST,
            "archive contains an unsupported entry",
        ),
        archive::ArchiveError::DuplicatePath => {
            (StatusCode::BAD_REQUEST, "archive contains a duplicate path")
        }
        archive::ArchiveError::FileTooLarge => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "archive contains an oversized file",
        ),
        archive::ArchiveError::Invalid | archive::ArchiveError::Empty => {
            (StatusCode::BAD_REQUEST, "import failed")
        }
    };
    error(status, message)
}
async fn project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = match parsed::<WorkspaceId>(&id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    project_response(&state, s.user_id, id).await
}
async fn files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = match parsed::<WorkspaceId>(&id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let access = match state.repo.project_access(s.user_id, id).await {
        Ok(value) => value,
        Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    if let ProjectAccess::Team { project, .. } = access {
        let canonical = match state.repo.team_files_for_user(s.user_id, project.id).await {
            Ok(value) => value,
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
        };
        let policies = canonical
            .into_iter()
            .map(|file| (file.path, file.policy))
            .collect::<BTreeMap<_, _>>();
        return match state.repo.private_working_tree(s.user_id, project.id).await {
            Ok(tree) => Json(
                tree.files
                    .into_iter()
                    .filter(|file| !file.pending_delete)
                    .map(|file| {
                        let policy = file
                            .canonical_path
                            .as_ref()
                            .and_then(|path| policies.get(path))
                            .copied()
                            .unwrap_or(FilePolicy::Editable);
                        FileWire {
                            path: file.path,
                            size_bytes: file.size_bytes,
                            revision: Some(file.canonical_revision.unwrap_or(0)),
                            policy: Some(policy.as_str().to_owned()),
                            draft_revision: file.draft_revision,
                            has_draft: file.added
                                || file.modified
                                || file.renamed
                                || file.draft_revision.is_some(),
                        }
                    })
                    .collect::<Vec<_>>(),
            )
            .into_response(),
            Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
        };
    }
    match state.workspaces.restore(id).await {
        Ok(v) => Json(
            v.files()
                .iter()
                .map(|(p, f)| FileWire {
                    path: p.as_str().to_owned(),
                    size_bytes: f.size_bytes(),
                    revision: None,
                    policy: None,
                    draft_revision: None,
                    has_draft: false,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"),
    }
}
async fn file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = match parsed::<WorkspaceId>(&id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let path = match LogicalPath::parse(&path) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid path"),
    };
    let access = match state.repo.project_access(s.user_id, id).await {
        Ok(value) => value,
        Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    if let ProjectAccess::Team { project, .. } = access {
        let projected = match state.repo.private_working_tree(s.user_id, project.id).await {
            Ok(tree) => tree
                .files
                .into_iter()
                .find(|file| file.path == path.as_str() && !file.pending_delete),
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
        };
        let Some(record) = projected else {
            return error(StatusCode::NOT_FOUND, "not found");
        };
        return match state.blobs.get(record.blob_hash).await {
            Ok(bytes) if u64::try_from(bytes.len()).ok() == Some(record.size_bytes) => {
                text_file_response(&state, id, bytes).await
            }
            Ok(_) | Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "blob storage failure"),
        };
    }
    match state.workspaces.read_file(id, &path).await {
        Ok(b) => text_file_response(&state, id, b).await,
        Err(_) => error(StatusCode::NOT_FOUND, "not found"),
    }
}
async fn put_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, path)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    };
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = match parsed::<WorkspaceId>(&id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let path = match LogicalPath::parse(&path) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid path"),
    };
    if body.len() > MAX_FILE_BYTES {
        return error(StatusCode::PAYLOAD_TOO_LARGE, "file too large");
    };
    let access = match state.repo.project_access(s.user_id, id).await {
        Ok(value) => value,
        Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    if let ProjectAccess::Team { project, .. } = access {
        let base = match file_revision(&headers) {
            Ok(value) => value,
            Err(response) => return response,
        };
        let expected_draft_revision = match draft_revision(&headers) {
            Ok(value) => value,
            Err(response) => return response,
        };
        let stored = match state.blobs.put(body).await {
            Ok(value) => value,
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "blob storage failure"),
        };
        return match state
            .repo
            .save_draft_with_revision(
                s.user_id,
                project.id,
                path.as_str(),
                base,
                stored.hash(),
                stored.size_bytes(),
                Some(expected_draft_revision),
            )
            .await
        {
            Ok(_) => Json(VersionWire {
                version: workspace_etag(&state, id)
                    .await
                    .trim_matches('"')
                    .parse()
                    .unwrap_or(0),
            })
            .into_response(),
            Err(AppError::Forbidden) => error(StatusCode::FORBIDDEN, "protected by project policy"),
            Err(AppError::DraftConflict) => {
                error(StatusCode::CONFLICT, "draft changed in another session")
            }
            Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
            Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
        };
    }
    let version = match if_match(&headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    match state
        .workspaces
        .put_file(id, s.user_id, version, path, body)
        .await
    {
        Ok(v) => Json(VersionWire { version: v.get() }).into_response(),
        Err(workspace_model::WorkspaceError::VersionConflict { .. }) => {
            error(StatusCode::CONFLICT, "stale workspace version")
        }
        Err(_) => error(StatusCode::BAD_REQUEST, "file update failed"),
    }
}
async fn delete_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, path)): Path<(String, String)>,
) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    };
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = match parsed::<WorkspaceId>(&id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let path = match LogicalPath::parse(&path) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid path"),
    };
    let access = match state.repo.project_access(s.user_id, id).await {
        Ok(value) => value,
        Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    if let ProjectAccess::Team { project, .. } = access {
        return match state
            .repo
            .delete_team_file(s.user_id, project.id, path.as_str())
            .await
        {
            Ok(version) => Json(VersionWire { version }).into_response(),
            Err(AppError::Forbidden) => error(StatusCode::FORBIDDEN, "protected by project policy"),
            Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
            Err(_) => error(StatusCode::CONFLICT, "team file deletion failed"),
        };
    }
    let version = match if_match(&headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    match state
        .workspaces
        .delete_file(id, s.user_id, version, path)
        .await
    {
        Ok(v) => Json(VersionWire { version: v.get() }).into_response(),
        Err(workspace_model::WorkspaceError::VersionConflict { .. }) => {
            error(StatusCode::CONFLICT, "stale workspace version")
        }
        Err(workspace_model::WorkspaceError::FileNotFound { .. }) => {
            error(StatusCode::NOT_FOUND, "file no longer exists")
        }
        Err(_) => error(StatusCode::BAD_REQUEST, "file delete failed"),
    }
}
async fn rename_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, from)): Path<(String, String)>,
    Json(input): Json<RenameFile>,
) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    };
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = match parsed::<WorkspaceId>(&id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let from = match LogicalPath::parse(&from) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid file path"),
    };
    let to = match LogicalPath::parse(&input.path) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid file path"),
    };
    let access = match state.repo.project_access(s.user_id, id).await {
        Ok(value) => value,
        Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    if let ProjectAccess::Team { project, .. } = access {
        return match state
            .repo
            .rename_team_file(s.user_id, project.id, from.as_str(), to.as_str())
            .await
        {
            Ok(version) => Json(VersionWire { version }).into_response(),
            Err(AppError::Forbidden) => error(StatusCode::FORBIDDEN, "protected by project policy"),
            Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
            Err(AppError::Conflict) => {
                error(StatusCode::CONFLICT, "destination file already exists")
            }
            Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
        };
    }
    let version = match if_match(&headers) {
        Ok(v) => v,
        Err(r) => return r,
    };
    match state
        .workspaces
        .rename_file(id, s.user_id, version, from, to)
        .await
    {
        Ok(v) => Json(VersionWire { version: v.get() }).into_response(),
        Err(workspace_model::WorkspaceError::VersionConflict { .. }) => {
            error(StatusCode::CONFLICT, "stale workspace version")
        }
        Err(workspace_model::WorkspaceError::FileAlreadyExists { .. }) => {
            error(StatusCode::CONFLICT, "destination file already exists")
        }
        Err(workspace_model::WorkspaceError::FileNotFound { .. }) => {
            error(StatusCode::NOT_FOUND, "file no longer exists")
        }
        Err(_) => error(StatusCode::BAD_REQUEST, "file rename failed"),
    }
}
async fn set_main(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<SetMain>,
) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    };
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = match parsed::<WorkspaceId>(&id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let path = match LogicalPath::parse(&input.path) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid path"),
    };
    if path.extension() != Some("tex") {
        return error(StatusCode::BAD_REQUEST, "main file must be a .tex file");
    }
    let access = match state.repo.project_access(s.user_id, id).await {
        Ok(value) => value,
        Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    if let ProjectAccess::Team { project, .. } = access {
        return match state
            .repo
            .set_team_main_file(s.user_id, project.id, path.as_str())
            .await
        {
            Ok(version) => Json(VersionWire { version }).into_response(),
            Err(AppError::Forbidden) => error(StatusCode::FORBIDDEN, "protected by project policy"),
            Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
            Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
        };
    }
    match state
        .workspaces
        .set_main_file(id, s.user_id, WorkspaceVersion::new(input.version), path)
        .await
    {
        Ok(v) => Json(VersionWire { version: v.get() }).into_response(),
        Err(workspace_model::WorkspaceError::VersionConflict { .. }) => {
            error(StatusCode::CONFLICT, "stale workspace version")
        }
        Err(_) => error(StatusCode::BAD_REQUEST, "main file update failed"),
    }
}
async fn submit_compile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(input): Json<CompileInput>,
) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    };
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = match parsed::<WorkspaceId>(&id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let permissions = match state.repo.effective_permissions(s.user_id, id).await {
        Ok(value) => value,
        Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    if !permissions.allows(persistence::Permission::CompileSubmit) {
        return error(
            StatusCode::FORBIDDEN,
            "compile capability required for this project",
        );
    }
    let checkpoint = match state.workspaces.force_snapshot(id).await {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "workspace must have a main file"),
    };
    let engine = input.engine.unwrap_or(TexEngine::PdfLatex);
    let synctex = input.synctex.unwrap_or(true);
    let profile = match LatexmkProfileId::parse("safe-v1") {
        Ok(v) => v,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "profile failure"),
    };
    let key = match CompileKeyMaterialV1::new(
        checkpoint.snapshot_id(),
        engine,
        state.environment.clone(),
        profile.clone(),
        ShellPolicy::Safe,
        synctex,
    )
    .compile_key()
    {
        Ok(v) => v,
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "compile key failure"),
    };
    let idem = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let idem = if idem.is_empty() {
        format!("web:{}", uuid::Uuid::new_v4())
    } else {
        idem.to_owned()
    };
    let idempotency = match IdempotencyKey::parse(&idem) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid idempotency key"),
    };
    let job = JobId::new();
    let req = EnqueueCompileJobV1 {
        job_id: job,
        tenant_id: s.tenant_id,
        user_id: s.user_id,
        workspace_id: id,
        snapshot_id: checkpoint.snapshot_id(),
        compile_key: key,
        idempotency_key: idempotency,
        engine,
        tex_environment_id: state.environment.clone(),
        latexmk_profile: profile,
        shell_policy: ShellPolicy::Safe,
        synctex,
        cost_class: CostClass::Normal,
        priority: 0,
    };
    match state.queue.enqueue(req).await {
        Ok(job) => job_response(&state, s.user_id, job).await,
        Err(_) => error(StatusCode::BAD_REQUEST, "compile admission failed"),
    }
}
async fn job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = match parsed::<JobId>(&id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    job_response(&state, s.user_id, id).await
}
async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    };
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = match parsed::<JobId>(&id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    if state.repo.job(s.user_id, id).await.is_err() {
        return error(StatusCode::NOT_FOUND, "not found");
    };
    match state
        .queue
        .request_cancellation(id, Some("cancelled by owner"))
        .await
    {
        Ok(()) => (StatusCode::NO_CONTENT).into_response(),
        Err(_) => error(StatusCode::CONFLICT, "job is terminal"),
    }
}
async fn artifacts(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let id = match parsed::<JobId>(&id) {
        Ok(v) => v,
        Err(r) => return r,
    };
    match state.repo.artifacts(s.user_id, id).await {
        Ok(v) => Json(
            v.into_iter()
                .map(|a| ArtifactWire {
                    id: a.id.to_string(),
                    name: a.logical_name,
                    size_bytes: a.size_bytes,
                    content_type: a.content_type,
                })
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn artifact(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((job, artifact)): Path<(String, String)>,
) -> Response {
    let s = match auth(&state, &headers).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let job = match parsed::<JobId>(&job) {
        Ok(v) => v,
        Err(r) => return r,
    };
    let artifact = match parsed::<ArtifactId>(&artifact) {
        Ok(v) => v,
        Err(r) => return r,
    };
    match state.repo.artifact(s.user_id, job, artifact).await {
        Ok(a) => match state.blobs.get(a.blob_hash).await {
            Ok(bytes) => {
                let disposition = if a.content_type == "application/pdf" {
                    "inline"
                } else {
                    "attachment"
                };
                (
                    [
                        (header::CONTENT_TYPE, a.content_type),
                        (
                            header::CONTENT_DISPOSITION,
                            format!(
                                "{disposition}; filename=\"{}\"",
                                a.logical_name.rsplit('/').next().unwrap_or("artifact")
                            ),
                        ),
                    ],
                    bytes,
                )
                    .into_response()
            }
            Err(_) => error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "artifact storage failure",
            ),
        },
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "personal and project-scoped collaboration response assembly must share one authorization boundary"
)]
async fn project_response(state: &AppState, user: UserId, id: WorkspaceId) -> Response {
    let access = match state.repo.project_access(user, id).await {
        Ok(value) => value,
        Err(AppError::NotFound) => return error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    };
    match state.workspaces.restore(id).await {
        Ok(v) => {
            let (name, files, collaboration, main_file) = match access {
                ProjectAccess::Personal { .. } => {
                    let project = match state.repo.project(user, id).await {
                        Ok(value) => value,
                        Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
                    };
                    let files = v
                        .files()
                        .iter()
                        .map(|(path, file)| FileWire {
                            path: path.as_str().to_owned(),
                            size_bytes: file.size_bytes(),
                            revision: None,
                            policy: None,
                            draft_revision: None,
                            has_draft: false,
                        })
                        .collect();
                    (
                        project.name,
                        files,
                        None,
                        v.main_file().map(|path| path.as_str().to_owned()),
                    )
                }
                ProjectAccess::ResearchGroup { group, .. } => {
                    let files = v
                        .files()
                        .iter()
                        .map(|(path, file)| FileWire {
                            path: path.as_str().to_owned(),
                            size_bytes: file.size_bytes(),
                            revision: None,
                            policy: None,
                            draft_revision: None,
                            has_draft: false,
                        })
                        .collect();
                    (
                        group.name,
                        files,
                        None,
                        v.main_file().map(|path| path.as_str().to_owned()),
                    )
                }
                ProjectAccess::Team {
                    project,
                    account_type,
                    can_write,
                    can_mentor,
                    can_manage,
                } => {
                    let canonical = match state.repo.team_files_for_user(user, project.id).await {
                        Ok(value) => value,
                        Err(_) => {
                            return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure");
                        }
                    };
                    let policies = canonical
                        .into_iter()
                        .map(|file| (file.path, file.policy))
                        .collect::<BTreeMap<_, _>>();
                    let tree = match state.repo.private_working_tree(user, project.id).await {
                        Ok(value) => value,
                        Err(_) => {
                            return error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure");
                        }
                    };
                    let main_file = tree
                        .pending_main
                        .or_else(|| v.main_file().map(|path| path.as_str().to_owned()));
                    let files = tree
                        .files
                        .into_iter()
                        .filter(|file| !file.pending_delete)
                        .map(|file| {
                            let policy = file
                                .canonical_path
                                .as_ref()
                                .and_then(|path| policies.get(path))
                                .copied()
                                .unwrap_or(FilePolicy::Editable);
                            FileWire {
                                path: file.path,
                                size_bytes: file.size_bytes,
                                revision: Some(file.canonical_revision.unwrap_or(0)),
                                policy: Some(policy.as_str().to_owned()),
                                draft_revision: file.draft_revision,
                                has_draft: file.added
                                    || file.modified
                                    || file.renamed
                                    || file.draft_revision.is_some(),
                            }
                        })
                        .collect();
                    (
                        project.name,
                        files,
                        Some(ProjectCollaborationWire {
                            team_id: project.team_id.to_string(),
                            team_project_id: project.id.to_string(),
                            canonical_generation: project.canonical_generation,
                            can_write,
                            can_mentor,
                            can_manage,
                            account_type: account_type.as_str().to_owned(),
                        }),
                        main_file,
                    )
                }
            };
            Json(ProjectWire {
                id: id.to_string(),
                name,
                version: v.version().get(),
                main_file,
                files,
                collaboration,
            })
            .into_response()
        }
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"),
    }
}
async fn job_response(state: &AppState, user: UserId, id: JobId) -> Response {
    match state.repo.job(user, id).await {
        Ok(j) => Json(JobWire {
            id: j.id.to_string(),
            project_id: j.workspace_id.to_string(),
            state: j.state,
            snapshot_id: j.snapshot_id,
            created_at: j.created_at,
            finished_at: j.finished_at,
            error: j.last_error,
            queue_position: j.queue_position,
            jobs_ahead: j.jobs_ahead,
        })
        .into_response(),
        Err(AppError::NotFound) => error(StatusCode::NOT_FOUND, "not found"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure"),
    }
}
async fn auth(state: &AppState, headers: &HeaderMap) -> Result<AppSessionRecord, Response> {
    let _principal = principal_auth(state, headers).await?;
    Err(error(
        StatusCode::FORBIDDEN,
        "the legacy browser product is retired",
    ))
}

async fn principal_auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedPrincipal, Response> {
    let principal = principal_auth_allow_temporary(state, headers).await?;
    if principal.session.must_change_password {
        return Err(error(
            StatusCode::PRECONDITION_REQUIRED,
            "password change required",
        ));
    }
    Ok(principal)
}

async fn principal_auth_allow_temporary(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AuthenticatedPrincipal, Response> {
    let token = cookie(headers)
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "authentication required"))?;
    let session = state
        .repo
        .session(&digest(&token))
        .await
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"))?
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "authentication required"))?;
    AuthenticatedPrincipal::resolve(session)
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"))
}
async fn session_response(
    state: &AppState,
    headers: &HeaderMap,
    user: UserId,
    email: String,
    must_change_password: bool,
) -> Response {
    let cookies = match create_session_headers(state, headers, user).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    (
        StatusCode::CREATED,
        cookies,
        Json(
            match identity_for_user(state, user, email, must_change_password).await {
                Ok(identity) => identity,
                Err(response) => return response,
            },
        ),
    )
        .into_response()
}

async fn create_session_headers(
    state: &AppState,
    headers: &HeaderMap,
    user: UserId,
) -> Result<HeaderMap, Response> {
    // Session rotation applies only to V2.  Legacy tokens were never part of the
    // V2 authentication contract and must not affect which session is created.
    for token in named_cookies(headers, COOKIE) {
        if state.repo.delete_session(&digest(&token)).await.is_err() {
            return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"));
        }
    }
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let token = hex::encode(bytes);
    if state
        .repo
        .create_session(&digest(&token), user, state.session_seconds)
        .await
        .is_err()
    {
        return Err(error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"));
    };
    session_cookie_headers(&token, state.session_seconds, state.cookie_secure)
        .map_err(|()| error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"))
}
async fn identity_response(state: &AppState, principal: AuthenticatedPrincipal) -> Response {
    match identity_for_kind(
        state,
        principal.user_id(),
        principal.email().to_owned(),
        principal.kind,
        principal.session.must_change_password,
    )
    .await
    {
        Ok(identity) => Json(identity).into_response(),
        Err(response) => response,
    }
}

async fn identity_for_user(
    state: &AppState,
    user: UserId,
    email: String,
    must_change_password: bool,
) -> Result<UserWire, Response> {
    let kind = if let Some(assignment) = state
        .v2
        .get_global_role(user)
        .await
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"))?
    {
        PrincipalKind::V2(assignment.role)
    } else {
        PrincipalKind::Legacy(
            state
                .repo
                .account_type(user)
                .await
                .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"))?,
        )
    };
    identity_for_kind(state, user, email, kind, must_change_password).await
}

async fn identity_for_kind(
    state: &AppState,
    user: UserId,
    email: String,
    kind: PrincipalKind,
    must_change_password: bool,
) -> Result<UserWire, Response> {
    let (account_type, persona, landing_path, v2_role, is_admin, has_mentor_projects) = match kind {
        PrincipalKind::V2(role) => (
            role.as_str(),
            role.as_str(),
            landing_path_for(kind, must_change_password),
            Some(role.as_str().to_owned()),
            role == GlobalRole::Admin,
            false,
        ),
        PrincipalKind::Legacy(account_type) => {
            let is_admin = account_type.is_admin();
            let has_mentor_projects = if is_admin {
                false
            } else {
                state
                    .repo
                    .has_mentor_project_role(user)
                    .await
                    .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"))?
            };
            let persona = if is_admin {
                "admin"
            } else if has_mentor_projects {
                "mentor"
            } else {
                account_type.as_str()
            };
            (
                account_type.as_str(),
                persona,
                landing_path_for(kind, must_change_password),
                None,
                is_admin,
                has_mentor_projects,
            )
        }
    };
    Ok(UserWire {
        id: user.to_string(),
        email,
        account_type: account_type.to_owned(),
        persona: persona.to_owned(),
        landing_path: landing_path.to_owned(),
        v2_role,
        capabilities: IdentityCapabilitiesWire {
            can_open_admin: is_admin,
            has_mentor_projects,
        },
        must_change_password,
    })
}

async fn principal_kind_for_user(
    state: &AppState,
    user: UserId,
) -> Result<PrincipalKind, Response> {
    if let Some(assignment) = state
        .v2
        .get_global_role(user)
        .await
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"))?
    {
        return Ok(PrincipalKind::V2(assignment.role));
    }
    state
        .repo
        .account_type(user)
        .await
        .map(PrincipalKind::Legacy)
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"))
}

const fn landing_path(kind: PrincipalKind) -> &'static str {
    match kind {
        PrincipalKind::V2(GlobalRole::Writer) => "/write",
        PrincipalKind::V2(GlobalRole::Mentor) => "/review",
        PrincipalKind::V2(GlobalRole::Admin) | PrincipalKind::Legacy(AccountType::Admin) => {
            "/admin"
        }
        PrincipalKind::Legacy(AccountType::Student | AccountType::Professor) => "/account-setup",
    }
}

const fn landing_path_for(kind: PrincipalKind, must_change_password: bool) -> &'static str {
    if must_change_password {
        "/change-password"
    } else {
        landing_path(kind)
    }
}

fn csrf(headers: &HeaderMap) -> Result<(), Response> {
    let Some(origin) = headers.get(header::ORIGIN) else {
        return Ok(());
    };
    let origin = origin
        .to_str()
        .map_err(|_| error(StatusCode::FORBIDDEN, "invalid origin"))?;
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| error(StatusCode::FORBIDDEN, "missing host"))?;
    if origin == format!("http://{host}") || origin == format!("https://{host}") {
        Ok(())
    } else {
        Err(error(StatusCode::FORBIDDEN, "cross-origin request denied"))
    }
}
fn cookie(headers: &HeaderMap) -> Option<String> {
    let tokens = named_cookies(headers, COOKIE);
    // A well-formed V2 browser has precisely one root-scoped cookie.  Refusing
    // an ambiguous manually crafted Cookie header is safer than guessing an order.
    (tokens.len() == 1).then(|| tokens[0].clone())
}
fn named_cookies(headers: &HeaderMap, name: &str) -> Vec<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .map(str::trim)
        .filter_map(|part| {
            part.strip_prefix(name)
                .and_then(|value| value.strip_prefix('='))
                .map(str::to_owned)
        })
        .collect()
}
fn session_cookie(name: &str, value: &str, path: &str, max_age: i64, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{name}={value}; Path={path}; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure}")
}
fn session_cookie_headers(
    token: &str,
    session_seconds: i64,
    secure: bool,
) -> Result<HeaderMap, ()> {
    let mut headers = HeaderMap::new();
    let value = HeaderValue::from_str(&session_cookie(COOKIE, token, "/", session_seconds, secure))
        .map_err(|_| ())?;
    headers.append(header::SET_COOKIE, value);
    for path in ["/"].into_iter().chain(LEGACY_COOKIE_PATHS) {
        let value = HeaderValue::from_str(&session_cookie(LEGACY_COOKIE, "", path, 0, secure))
            .map_err(|_| ())?;
        headers.append(header::SET_COOKIE, value);
    }
    Ok(headers)
}
fn expired_session_cookie_headers(secure: bool) -> Result<HeaderMap, ()> {
    let mut headers = HeaderMap::new();
    let canonical =
        HeaderValue::from_str(&session_cookie(COOKIE, "", "/", 0, secure)).map_err(|_| ())?;
    headers.append(header::SET_COOKIE, canonical);
    for path in ["/"].into_iter().chain(LEGACY_COOKIE_PATHS) {
        let value = HeaderValue::from_str(&session_cookie(LEGACY_COOKIE, "", path, 0, secure))
            .map_err(|_| ())?;
        headers.append(header::SET_COOKIE, value);
    }
    Ok(headers)
}
fn digest(value: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn if_match(headers: &HeaderMap) -> Result<WorkspaceVersion, Response> {
    let value = headers
        .get(header::IF_MATCH)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| error(StatusCode::PRECONDITION_REQUIRED, "If-Match required"))?;
    let value = value
        .trim_matches('"')
        .parse::<u64>()
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid If-Match"))?;
    Ok(WorkspaceVersion::new(value))
}
fn file_revision(headers: &HeaderMap) -> Result<u64, Response> {
    headers
        .get("x-file-revision")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            error(
                StatusCode::PRECONDITION_REQUIRED,
                "X-File-Revision required",
            )
        })?
        .parse()
        .map_err(|_| error(StatusCode::BAD_REQUEST, "invalid X-File-Revision"))
}
fn draft_revision(headers: &HeaderMap) -> Result<u64, Response> {
    headers
        .get("if-draft-match")
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "private save revision is required"))?
        .to_str()
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "invalid private save revision"))
}
async fn text_file_response(state: &AppState, id: WorkspaceId, bytes: Bytes) -> Response {
    let mut response = bytes.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    match HeaderValue::from_str(&workspace_etag(state, id).await) {
        Ok(value) => {
            response.headers_mut().insert(header::ETAG, value);
            response
        }
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "header failure"),
    }
}
async fn workspace_etag(state: &AppState, id: WorkspaceId) -> String {
    state.workspaces.restore(id).await.map_or_else(
        |_| "\"0\"".to_owned(),
        |v| format!("\"{}\"", v.version().get()),
    )
}
fn parsed<T: FromStr>(value: &str) -> Result<T, Response> {
    value
        .parse()
        .map_err(|_| error(StatusCode::NOT_FOUND, "not found"))
}
fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ErrorWire {
            error: message.into(),
        }),
    )
        .into_response()
}
fn required(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    env::var(name).map_err(|_| format!("required environment variable {name} is missing").into())
}
fn int_env(name: &str, default: i64) -> Result<i64, Box<dyn std::error::Error>> {
    match env::var(name) {
        Ok(v) => {
            let n = v.parse()?;
            if n <= 0 {
                return Err(format!("{name} must be positive").into());
            }
            Ok(n)
        }
        Err(_) => Ok(default),
    }
}
fn bool_env(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
fn queue_limits() -> Result<QueueLimits, Box<dyn std::error::Error>> {
    QueueLimits::new(
        u32::try_from(int_env("QUEUE_GLOBAL_RUNNING", 2)?)?,
        u32::try_from(int_env("QUEUE_PER_USER_RUNNING", 1)?)?,
        u32::try_from(int_env("QUEUE_PER_USER_OUTSTANDING", 8)?)?,
        Duration::from_secs(u64::try_from(int_env("QUEUE_LEASE_SECONDS", 120)?)?),
        u32::try_from(int_env("QUEUE_MAX_ATTEMPTS", 3)?)?,
    )
    .map_err(Into::into)
}

fn login_html(error_message: Option<&str>) -> String {
    include_str!("ui.html")
        .replacen("<body>", "<body data-server-authenticated=\"false\">", 1)
        .replace("{{LOGIN_ERROR}}", error_message.unwrap_or(""))
}

fn writer_html() -> &'static str {
    include_str!("write.html")
}

fn mentor_html() -> &'static str {
    include_str!("review.html")
}

fn admin_html() -> &'static str {
    include_str!("admin.html")
}

fn account_setup_html() -> &'static str {
    "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Account setup required — LaTeX Core</title><link rel=\"stylesheet\" href=\"/static/styles.css?v=cutover\"></head><body><main class=\"login-view\"><section class=\"login-card\"><div class=\"wordmark\">LaTeX Core</div><h1>Account setup required</h1><p>This account has not yet been assigned a Writer, Mentor, or Admin role.</p><form method=\"post\" action=\"/logout\"><button class=\"primary\" type=\"submit\">Log out</button></form></section></main></body></html>"
}

fn change_password_html(error_message: Option<&str>) -> String {
    let error = error_message.unwrap_or("");
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Set your password — LaTeX Core</title><link rel=\"stylesheet\" href=\"/static/styles.css?v=v2-2-password\"></head><body><main class=\"login-view\"><section class=\"login-card\"><div class=\"wordmark\">LaTeX Core</div><h1>Set your password</h1><p>Your temporary password worked. Choose a new password before continuing.</p><p class=\"danger\" role=\"alert\">{error}</p><form method=\"post\" action=\"/change-password\"><label>New password<input name=\"new_password\" type=\"password\" minlength=\"12\" maxlength=\"256\" required autocomplete=\"new-password\"></label><label>Confirm password<input name=\"confirm_password\" type=\"password\" minlength=\"12\" maxlength=\"256\" required autocomplete=\"new-password\"></label><button class=\"primary\" type=\"submit\">Set password</button></form><form method=\"post\" action=\"/logout\"><button type=\"submit\">Log out</button></form></section></main></body></html>"
    )
}

fn redirect_with_cookies(location: &'static str, cookies: HeaderMap) -> Response {
    let mut response = (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response();
    response.headers_mut().extend(cookies);
    response
}

async fn ui(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match principal_auth_allow_temporary(&state, &headers).await {
        Ok(value) => value,
        Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
            return Html(login_html(None)).into_response();
        }
        Err(response) => return response,
    };
    redirect_with_cookies(
        landing_path_for(principal.kind, principal.session.must_change_password),
        HeaderMap::new(),
    )
}

async fn admin_ui(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match principal_auth_allow_temporary(&state, &headers).await {
        Ok(value) => value,
        Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
            return Html(login_html(None)).into_response();
        }
        Err(response) => return response,
    };
    if principal.session.must_change_password {
        return redirect_with_cookies("/change-password", HeaderMap::new());
    }
    match principal.kind {
        PrincipalKind::V2(GlobalRole::Admin) | PrincipalKind::Legacy(AccountType::Admin) => {
            Html(admin_html()).into_response()
        }
        PrincipalKind::V2(_) | PrincipalKind::Legacy(_) => {
            error(StatusCode::FORBIDDEN, "administrative access required")
        }
    }
}

async fn writer_ui(State(state): State<AppState>, headers: HeaderMap) -> Response {
    role_ui(&state, &headers, GlobalRole::Writer, writer_html()).await
}

async fn mentor_ui(State(state): State<AppState>, headers: HeaderMap) -> Response {
    role_ui(&state, &headers, GlobalRole::Mentor, mentor_html()).await
}

async fn account_setup_ui(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match principal_auth_allow_temporary(&state, &headers).await {
        Ok(value) => value,
        Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
            return Html(login_html(None)).into_response();
        }
        Err(response) => return response,
    };
    if principal.session.must_change_password {
        return redirect_with_cookies("/change-password", HeaderMap::new());
    }
    match principal.kind {
        PrincipalKind::Legacy(AccountType::Student | AccountType::Professor) => {
            Html(account_setup_html()).into_response()
        }
        PrincipalKind::V2(_) | PrincipalKind::Legacy(AccountType::Admin) => {
            redirect_with_cookies(landing_path(principal.kind), HeaderMap::new())
        }
    }
}

async fn role_ui(
    state: &AppState,
    headers: &HeaderMap,
    required: GlobalRole,
    html: &'static str,
) -> Response {
    let principal = match principal_auth_allow_temporary(state, headers).await {
        Ok(value) => value,
        Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
            return Html(login_html(None)).into_response();
        }
        Err(response) => return response,
    };
    if principal.session.must_change_password {
        return redirect_with_cookies("/change-password", HeaderMap::new());
    }
    match principal.kind {
        PrincipalKind::V2(role) if role == required => Html(html).into_response(),
        PrincipalKind::V2(_) | PrincipalKind::Legacy(_) => {
            error(StatusCode::FORBIDDEN, "role-specific access required")
        }
    }
}

async fn change_password_ui(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match principal_auth_allow_temporary(&state, &headers).await {
        Ok(value) => value,
        Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
            return Html(login_html(None)).into_response();
        }
        Err(response) => return response,
    };
    if principal.session.must_change_password {
        Html(change_password_html(None)).into_response()
    } else {
        redirect_with_cookies(landing_path(principal.kind), HeaderMap::new())
    }
}

async fn workspace_ui(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match principal_auth(&state, &headers).await {
        Ok(_) => error(StatusCode::FORBIDDEN, "the legacy workspace is retired"),
        Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
            Html(login_html(None)).into_response()
        }
        Err(response) => response,
    }
}

async fn styles() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/styles.css"),
    )
        .into_response()
}

async fn shells_css() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/shells.css"),
    )
        .into_response()
}

async fn admin_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/admin.js"),
    )
        .into_response()
}

async fn writer_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/writer.js"),
    )
        .into_response()
}

async fn app_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/app.js"),
    )
        .into_response()
}

async fn api_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/api.js"),
    )
        .into_response()
}

async fn state_js() -> Response {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../static/state.js"),
    )
        .into_response()
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "unit assertion fixture")]
mod tests {
    use super::*;

    #[test]
    fn browser_login_and_account_setup_are_server_rendered_and_minimal() {
        let login = login_html(Some("Invalid email or password."));
        assert!(login.contains("method=\"post\" action=\"/login\""));
        assert!(login.contains("name=\"email\""));
        assert!(login.contains("name=\"password\""));
        assert!(login.contains("Invalid email or password."));

        let setup = account_setup_html();
        assert!(setup.contains("Account setup required"));
        assert!(setup.contains("has not yet been assigned"));
        assert!(setup.contains("method=\"post\" action=\"/logout\""));
        for forbidden in ["/workspace", "/write", "/review", "/admin", "app.js"] {
            assert!(!setup.contains(forbidden));
        }
    }

    #[test]
    fn v2_bootstrap_document_produces_a_pdf_page() {
        assert!(INITIAL_TEX.contains("\\begin{document}\nStart writing your paper."));
        assert!(!INITIAL_TEX.contains("\\begin{document}\n\n\\end{document}"));
    }

    #[test]
    fn browser_login_redirect_has_a_canonical_session_cookie() {
        let response = redirect_with_cookies(
            "/",
            session_cookie_headers("token", 60, false).expect("valid cookie headers"),
        );
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::LOCATION], "/");
        assert!(
            response
                .headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .any(|value| value
                    .as_bytes()
                    .starts_with(b"latex_core_session_v2=token; Path=/; HttpOnly; SameSite=Lax"))
        );
    }

    #[test]
    fn v2_cookie_parser_ignores_legacy_and_unrelated_cookie_values() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static(
                "other=x; latex_core_session=legacy; x=y; latex_core_session_v2=token",
            ),
        );
        assert_eq!(cookie(&headers).as_deref(), Some("token"));
    }

    #[test]
    fn v2_cookie_parser_rejects_ambiguous_v2_values_and_ignores_multiple_legacy_values() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("latex_core_session=old-a; latex_core_session=old-b"),
        );
        assert_eq!(cookie(&headers), None);
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("latex_core_session_v2=one; latex_core_session_v2=two"),
        );
        assert_eq!(cookie(&headers), None);
    }

    #[test]
    fn session_cookie_headers_use_compatible_canonical_scope() {
        let login = session_cookie_headers("token", 60, true).expect("valid cookie headers");
        let login_values = login
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().expect("valid header value"))
            .collect::<Vec<_>>();
        assert_eq!(
            login_values[0],
            "latex_core_session_v2=token; Path=/; HttpOnly; SameSite=Lax; Max-Age=60; Secure"
        );
        assert!(
            login_values.contains(
                &"latex_core_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Secure"
            )
        );
        assert!(login_values.contains(
            &"latex_core_session=; Path=/api; HttpOnly; SameSite=Lax; Max-Age=0; Secure"
        ));

        let logout = expired_session_cookie_headers(true).expect("valid cookie headers");
        let logout_values = logout
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().expect("valid header value"))
            .collect::<Vec<_>>();
        assert_eq!(
            logout_values[0],
            "latex_core_session_v2=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0; Secure"
        );
        assert!(logout_values.contains(
            &"latex_core_session=; Path=/api/auth; HttpOnly; SameSite=Lax; Max-Age=0; Secure"
        ));
        assert_eq!(
            session_cookie(COOKIE, "token", "/", 60, false),
            "latex_core_session_v2=token; Path=/; HttpOnly; SameSite=Lax; Max-Age=60"
        );
    }

    #[test]
    fn csrf_rejects_a_cross_origin_post() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("latex.example"));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://attacker.example"),
        );
        assert!(csrf(&headers).is_err());
    }

    #[test]
    fn logical_paths_and_stale_versions_are_strict() {
        assert!(LogicalPath::parse("../escape.tex").is_err());
        let mut headers = HeaderMap::new();
        headers.insert(header::IF_MATCH, HeaderValue::from_static("\"7\""));
        assert_eq!(if_match(&headers).expect("valid ETag").get(), 7);
        headers.insert(header::IF_MATCH, HeaderValue::from_static("\"NaN\""));
        assert!(if_match(&headers).is_err());
    }

    #[test]
    fn v2_shells_expose_the_compact_three_pane_information_architecture_only() {
        let writer = writer_html();
        for required in [
            "PAPERS",
            "TEAM PAPERS",
            "FILES",
            "SOURCE",
            "PDF",
            "workspaceDrawer",
            "Math palette",
            "Send for Review",
        ] {
            assert!(
                writer.contains(required),
                "missing Writer section {required}"
            );
        }
        for forbidden in [
            "Research Groups",
            "Create Team",
            "New Team",
            "Teams +",
            "Project Manager",
            "Publish Changes",
            "Add Member",
        ] {
            assert!(
                !writer.contains(forbidden),
                "unexpected Writer control {forbidden}"
            );
        }

        let mentor = mentor_html();
        for required in [
            "ASSIGNED PAPERS",
            "READ-ONLY SOURCE",
            "Waiting for Team Review",
            "reviewPopover",
            "Suggest replacement",
        ] {
            assert!(
                mentor.contains(required),
                "missing Mentor section {required}"
            );
        }
        for forbidden in [
            "Research Groups",
            "Create Team",
            "New Team",
            "Teams +",
            "Project Manager",
            ">Save<",
            "Set Main",
            "New File",
            "Rename",
            "Move",
            "Delete",
            "Publish",
            "ACTIVITY",
            "CHANGES SINCE",
            "RESTORATION REQUESTS",
            "APPROVALS",
            "NEW ANNOTATION",
        ] {
            assert!(
                !mentor.contains(forbidden),
                "unexpected Mentor control {forbidden}"
            );
        }

        let admin = admin_html();
        assert!(!admin.contains("/workspace"));
        assert!(!admin.contains("/write"));
        assert!(!admin.contains("/review"));
        assert!(!admin.contains("<textarea"));
        assert!(!admin.contains(">Workspace<"));
        for required in [
            "OVERVIEW",
            "V2 USERS",
            "INSTITUTION DATA",
            "IMPORTS",
            "PAPER TEAMS",
            "TEMPLATES",
            "FILE POLICIES",
            "VERSIONS",
            "REVIEWS",
            "BUILD QUEUE",
            "AUDIT",
            "SYSTEM",
        ] {
            assert!(admin.contains(required), "missing Admin section {required}");
        }
        assert!(!admin.contains("LEGACY RESEARCH GROUPS"));
        assert!(!admin.contains("PROGRAMME TEMPLATES"));
    }

    #[test]
    fn institution_error_csv_neutralizes_spreadsheet_formulas() {
        assert_eq!(csv_escape("=2+2"), "'=2+2");
        assert_eq!(csv_escape("+cmd"), "'+cmd");
        assert_eq!(csv_escape("-1"), "'-1");
        assert_eq!(csv_escape("@SUM(A1:A2)"), "'@SUM(A1:A2)");
        assert_eq!(csv_escape("safe,value"), "\"safe,value\"");
    }
}

#[cfg(all(test, feature = "database-tests"))]
#[allow(
    clippy::expect_used,
    clippy::similar_names,
    clippy::too_many_lines,
    clippy::unwrap_used,
    reason = "disposable PostgreSQL authorization fixtures"
)]
mod database_tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Method, Request},
    };
    use persistence::{DatabaseConfig, GlobalRole};
    use sqlx::PgPool;
    use std::io::{Cursor, Write};
    use tempfile::TempDir;
    use tower::ServiceExt;

    static SERVER_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    const PASSWORD: &str = "CorrectHorseBattery1";

    struct Fixture {
        email: String,
        cookie: String,
    }

    #[tokio::test]
    async fn v2_and_legacy_authorization_route_login_api_and_mutation_matrix() {
        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, _state) = test_application().await;

        let writer = fixture(&app, &database, "admin", Some(GlobalRole::Writer)).await;
        let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let admin = fixture(&app, &database, "student", Some(GlobalRole::Admin)).await;
        let legacy_student = fixture(&app, &database, "student", None).await;
        let legacy_professor = fixture(&app, &database, "professor", None).await;
        let legacy_admin = fixture(&app, &database, "admin", None).await;

        assert_login_redirect(&app, &writer.email, "/write").await;
        assert_login_redirect(&app, &mentor.email, "/review").await;
        assert_login_redirect(&app, &admin.email, "/admin").await;
        assert_login_redirect(&app, &legacy_student.email, "/account-setup").await;
        assert_login_redirect(&app, &legacy_professor.email, "/account-setup").await;
        assert_login_redirect(&app, &legacy_admin.email, "/admin").await;

        assert_routes(
            &app,
            &writer.cookie,
            &[
                ("/write", 200),
                ("/review", 403),
                ("/admin", 403),
                ("/account-setup", 303),
                ("/workspace", 403),
            ],
        )
        .await;
        assert_routes(
            &app,
            &mentor.cookie,
            &[
                ("/write", 403),
                ("/review", 200),
                ("/admin", 403),
                ("/account-setup", 303),
                ("/workspace", 403),
            ],
        )
        .await;
        assert_routes(
            &app,
            &admin.cookie,
            &[
                ("/write", 403),
                ("/review", 403),
                ("/admin", 200),
                ("/account-setup", 303),
                ("/workspace", 403),
            ],
        )
        .await;
        assert_routes(
            &app,
            &legacy_student.cookie,
            &[("/", 303), ("/account-setup", 200), ("/workspace", 403)],
        )
        .await;
        assert_routes(
            &app,
            &legacy_professor.cookie,
            &[("/", 303), ("/account-setup", 200), ("/workspace", 403)],
        )
        .await;
        assert_routes(
            &app,
            &legacy_admin.cookie,
            &[
                ("/admin", 200),
                ("/account-setup", 303),
                ("/workspace", 403),
            ],
        )
        .await;

        assert_eq!(
            get(&app, "/api/projects", None).await.status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            get(&app, "/api/admin/overview", Some(&writer.cookie))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            get(&app, "/api/admin/overview", Some(&mentor.cookie))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            get(&app, "/api/admin/overview", Some(&admin.cookie))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            get(&app, "/api/admin/overview", Some(&legacy_admin.cookie))
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            get(&app, "/api/v2/me", Some(&legacy_student.cookie))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            get(&app, "/api/v2/me", Some(&writer.cookie)).await.status(),
            StatusCode::OK
        );

        for fixture in [
            &writer,
            &mentor,
            &admin,
            &legacy_student,
            &legacy_professor,
            &legacy_admin,
        ] {
            let workspace = WorkspaceId::new();
            let group = uuid::Uuid::new_v4();
            let team_project = uuid::Uuid::new_v4();
            for (method, path, body, content_type) in [
                (
                    Method::POST,
                    "/api/projects".to_owned(),
                    r#"{"name":"legacy bypass"}"#,
                    "application/json",
                ),
                (
                    Method::PUT,
                    format!("/api/projects/{workspace}/files/main.tex"),
                    "source",
                    "text/plain",
                ),
                (
                    Method::POST,
                    "/api/teams".to_owned(),
                    r#"{"name":"legacy team"}"#,
                    "application/json",
                ),
                (
                    Method::POST,
                    "/api/research-groups".to_owned(),
                    r#"{"name":"legacy group"}"#,
                    "application/json",
                ),
                (
                    Method::PATCH,
                    format!("/api/research-groups/{group}"),
                    r#"{"name":"changed"}"#,
                    "application/json",
                ),
                (
                    Method::POST,
                    format!("/api/team-projects/{team_project}/publish"),
                    "{}",
                    "application/json",
                ),
            ] {
                assert_eq!(
                    request(
                        &app,
                        method,
                        &path,
                        Some(&fixture.cookie),
                        body,
                        Some(content_type)
                    )
                    .await
                    .status(),
                    StatusCode::FORBIDDEN,
                    "legacy route {path} leaked for {}",
                    fixture.email
                );
            }
        }

        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM latex_core.global_user_roles r \
                 JOIN latex_core.user_credentials c ON c.user_id=r.user_id \
                 WHERE c.email IN ($1,$2,$3,$4,$5,$6)",
            )
            .bind(&writer.email)
            .bind(&mentor.email)
            .bind(&admin.email)
            .bind(&legacy_student.email)
            .bind(&legacy_professor.email)
            .bind(&legacy_admin.email)
            .fetch_one(&pool)
            .await
            .unwrap(),
            3
        );
        let users = test_json(get(&app, "/api/admin/v2/users", Some(&admin.cookie)).await).await;
        let unassigned = users
            .as_array()
            .unwrap()
            .iter()
            .find(|user| user["email"].as_str() == Some(legacy_student.email.as_str()))
            .unwrap();
        assert_eq!(unassigned["legacy_account_type"], "student");
        assert!(unassigned["v2_role"].is_null());
        assert_eq!(unassigned["migration_state"], "UNASSIGNED");
        let legacy_student_id = test_user_id(&pool, &legacy_student.email).await;
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &format!("/api/admin/v2/users/{legacy_student_id}/role"),
                Some(&admin.cookie),
                r#"{"role":"writer"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            get(&app, "/api/v2/me", Some(&legacy_student.cookie))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_login_redirect(&app, &legacy_student.email, "/write").await;
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM latex_core.global_user_roles WHERE user_id IN ((SELECT user_id FROM latex_core.user_credentials WHERE email=$1),(SELECT user_id FROM latex_core.user_credentials WHERE email=$2))")
            .bind(&legacy_professor.email).bind(&legacy_admin.email).fetch_one(&pool).await.unwrap(), 0);

        let invalid = login_request(&app, "missing@example.test", "wrong").await;
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
        let logout = request(
            &app,
            Method::POST,
            "/logout",
            Some(&mentor.cookie),
            "",
            None,
        )
        .await;
        assert_eq!(logout.status(), StatusCode::SEE_OTHER);
        assert_eq!(logout.headers()[header::LOCATION], "/");
        assert_eq!(
            get(&app, "/api/v2/me", Some(&mentor.cookie)).await.status(),
            StatusCode::UNAUTHORIZED
        );

        pool.close().await;
        database.close().await;
    }

    #[tokio::test]
    async fn temporary_password_requires_first_login_change_before_role_access() {
        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, _state) = test_application().await;
        let repo = AppRepository::new(database.clone());
        let email = format!("{}@temporary.example", uuid::Uuid::new_v4());
        let temporary_password = auth::temporary_password();
        let temporary_hash = auth::hash_temporary_password(&temporary_password).unwrap();
        repo.create_v2_account(&email, &temporary_hash, GlobalRole::Writer, true)
            .await
            .unwrap();

        let login = login_request(&app, &email, &temporary_password).await;
        assert_eq!(login.status(), StatusCode::SEE_OTHER);
        assert_eq!(login.headers()[header::LOCATION], "/change-password");
        let temporary_cookie = response_cookie(&login);
        let write = get(&app, "/write", Some(&temporary_cookie)).await;
        assert_eq!(write.status(), StatusCode::SEE_OTHER);
        assert_eq!(write.headers()[header::LOCATION], "/change-password");
        assert_eq!(
            get(&app, "/api/v2/writer/papers", Some(&temporary_cookie))
                .await
                .status(),
            StatusCode::PRECONDITION_REQUIRED
        );
        assert_eq!(
            get(&app, "/change-password", Some(&temporary_cookie))
                .await
                .status(),
            StatusCode::OK
        );

        let new_password = "A-New-Permanent-Password-2026";
        let changed = request(
            &app,
            Method::POST,
            "/change-password",
            Some(&temporary_cookie),
            &format!("new_password={new_password}&confirm_password={new_password}"),
            Some("application/x-www-form-urlencoded"),
        )
        .await;
        assert_eq!(changed.status(), StatusCode::SEE_OTHER);
        assert_eq!(changed.headers()[header::LOCATION], "/write");
        let active_cookie = response_cookie(&changed);
        assert_eq!(
            get(&app, "/write", Some(&active_cookie)).await.status(),
            StatusCode::OK
        );
        assert!(
            !sqlx::query_scalar::<_, bool>(
                "SELECT must_change_password FROM latex_core.user_credentials WHERE email=$1",
            )
            .bind(&email)
            .fetch_one(&pool)
            .await
            .unwrap()
        );
        assert_eq!(
            login_request(&app, &email, &temporary_password)
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let permanent_login = login_request(&app, &email, new_password).await;
        assert_eq!(permanent_login.status(), StatusCode::SEE_OTHER);
        assert_eq!(permanent_login.headers()[header::LOCATION], "/write");

        pool.close().await;
        database.close().await;
    }

    #[tokio::test]
    async fn one_add_batch_provisions_accounts_and_materializes_a_team() {
        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, state) = test_application().await;
        let admin = fixture(&app, &database, "student", Some(GlobalRole::Admin)).await;
        let admin_id = test_user_id(&pool, &admin.email).await;
        let template_id = uuid::Uuid::new_v4();
        let main = state
            .blobs
            .put(Bytes::from_static(b"\\documentclass{article}\n"))
            .await
            .unwrap();
        state
            .repo
            .create_template(
                template_id,
                "Institution default",
                None,
                Some("main.tex"),
                &[AppTemplateFileRecord {
                    path: "main.tex".into(),
                    blob_hash: main.hash(),
                    size_bytes: main.size_bytes(),
                }],
            )
            .await
            .unwrap();
        state
            .institution
            .set_global_fallback(admin_id, template_id)
            .await
            .unwrap();

        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let department_id = uuid::Uuid::new_v4();
        let programme = format!("P{suffix}");
        let faculty_id = format!("F{suffix}");
        let reg_no = format!("S{suffix}");
        let external_team_key = format!("T{suffix}");
        let student_email = format!("newstudent-{suffix}@example.edu");
        let mentor_email = format!("newmentor-{suffix}@example.edu");
        let files = vec![
            ("departments.csv", format!("department_id\n{department_id}\n").into_bytes()),
            ("faculty.csv", format!("faculty_id,name,email,dept_id,honorific,designation,status\n{faculty_id},New Mentor,{mentor_email},{department_id},Dr,Mentor,ACTIVE\n").into_bytes()),
            ("programmes.csv", format!("programme_code,hod_id\n{programme},{faculty_id}\n").into_bytes()),
            ("students.csv", format!("reg_no,name,email,programme_code\n{reg_no},New Student,{student_email},{programme}\n").into_bytes()),
            ("paper_teams.csv", format!("external_team_key,team_name,academic_year,semester,status\n{external_team_key},Imported Team,2026,1,ACTIVE\n").into_bytes()),
            ("paper_team_writers.csv", format!("external_team_key,student_reg_no,writer_order,is_leader\n{external_team_key},{reg_no},1,true\n").into_bytes()),
            ("paper_team_mentors.csv", format!("external_team_key,faculty_id\n{external_team_key},{faculty_id}\n").into_bytes()),
        ];
        let (multipart, content_type) = institution_batch_multipart(&files);
        let validated = test_json(
            request_bytes(
                &app,
                Method::POST,
                "/api/admin/v2/institution/import-batches/validate",
                Some(&admin.cookie),
                multipart,
                Some(&content_type),
            )
            .await,
        )
        .await;
        assert_eq!(validated["batch"]["status"], "VALIDATED");
        let batch_id = validated["batch"]["id"].as_str().unwrap();
        let applied = test_json(
            request(
                &app,
                Method::POST,
                &format!("/api/admin/v2/institution/import-batches/{batch_id}/apply"),
                Some(&admin.cookie),
                "{}",
                Some("application/json"),
            )
            .await,
        )
        .await;
        let credentials = applied["account_provisioning"]["credentials"]
            .as_array()
            .unwrap();
        assert_eq!(credentials.len(), 2);
        assert_eq!(applied["account_provisioning"]["created"], 2);
        let paper_team_id: uuid::Uuid = sqlx::query_scalar("SELECT paper_team_id FROM latex_core.external_paper_team_links WHERE external_team_key=$1").bind(&external_team_key).fetch_one(&pool).await.unwrap();
        let members: Vec<(String, String, bool, Option<i32>)> = sqlx::query_as("SELECT credentials.email,role.role,member.is_leader,member.writer_order FROM latex_core.paper_team_members member JOIN latex_core.user_credentials credentials ON credentials.user_id=member.user_id JOIN latex_core.global_user_roles role ON role.user_id=member.user_id WHERE member.paper_team_id=$1 ORDER BY member.writer_order NULLS LAST").bind(paper_team_id).fetch_all(&pool).await.unwrap();
        assert_eq!(
            members,
            vec![
                (student_email.clone(), "writer".into(), true, Some(1)),
                (mentor_email.clone(), "mentor".into(), false, None)
            ]
        );
        let pinned: uuid::Uuid = sqlx::query_scalar(
            "SELECT template_id FROM latex_core.paper_template_pins WHERE paper_id=$1",
        )
        .bind(paper_team_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pinned, template_id);

        for credential in credentials {
            let email = credential["email"].as_str().unwrap();
            let temporary = credential["temporary_password"].as_str().unwrap();
            assert_eq!(temporary.len(), 8);
            let login = login_request(&app, email, temporary).await;
            assert_eq!(login.headers()[header::LOCATION], "/change-password");
            let temporary_cookie = response_cookie(&login);
            let new_password = format!(
                "Permanent-{suffix}-{}",
                credential["credential_role"].as_str().unwrap()
            );
            let changed = request(
                &app,
                Method::POST,
                "/change-password",
                Some(&temporary_cookie),
                &format!("new_password={new_password}&confirm_password={new_password}"),
                Some("application/x-www-form-urlencoded"),
            )
            .await;
            let expected_path = if credential["credential_role"] == "student" {
                "/write"
            } else {
                "/review"
            };
            assert_eq!(changed.headers()[header::LOCATION], expected_path);
            let active_cookie = response_cookie(&changed);
            let papers_path = if credential["credential_role"] == "student" {
                "/api/v2/writer/papers"
            } else {
                "/api/v2/mentor/papers"
            };
            let papers = test_json(get(&app, papers_path, Some(&active_cookie)).await).await;
            assert!(papers.to_string().contains(&paper_team_id.to_string()));
        }

        pool.close().await;
        database.close().await;
    }

    #[tokio::test]
    async fn admin_temporary_password_reset_is_one_time_and_revokes_sessions() {
        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, _state) = test_application().await;
        let admin = fixture(&app, &database, "student", Some(GlobalRole::Admin)).await;
        let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let writer_id = test_user_id(&pool, &writer.email).await;
        let reset = test_json(
            request(
                &app,
                Method::POST,
                &format!("/api/admin/v2/users/{writer_id}/temporary-password"),
                Some(&admin.cookie),
                "{}",
                Some("application/json"),
            )
            .await,
        )
        .await;
        let temporary = reset["temporary_password"].as_str().unwrap();
        assert_eq!(temporary.len(), 8);
        assert_eq!(reset["must_change_password"], true);
        assert_eq!(
            get(&app, "/api/v2/writer/papers", Some(&writer.cookie))
                .await
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let stored: (String, bool) = sqlx::query_as(
            "SELECT password_hash,must_change_password FROM latex_core.user_credentials WHERE user_id=$1",
        )
        .bind(writer_id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(stored.0.starts_with("$argon2"));
        assert!(!stored.0.contains(temporary));
        assert!(stored.1);
        let audit: (String, String) = sqlx::query_as(
            "SELECT event_type,metadata::text FROM latex_core.audit_events \
             WHERE resource_id=$1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(writer_id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(audit.0, "account.temporary_password.generated");
        assert!(!audit.1.contains(temporary));
        let login = login_request(&app, &writer.email, temporary).await;
        assert_eq!(login.headers()[header::LOCATION], "/change-password");

        pool.close().await;
        database.close().await;
    }

    #[tokio::test]
    async fn s2_admin_team_writer_and_file_vertical_slice() {
        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, _state) = test_application().await;
        let legacy_admin = fixture(&app, &database, "admin", None).await;
        let admin = fixture(&app, &database, "student", Some(GlobalRole::Admin)).await;
        let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let other_writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let mentor = fixture(&app, &database, "student", Some(GlobalRole::Mentor)).await;

        let email = format!("{}@s2.example", uuid::Uuid::new_v4());
        let create_user = request(
            &app,
            Method::POST,
            "/api/admin/v2/users",
            Some(&legacy_admin.cookie),
            &serde_json::json!({"email":email,"password":PASSWORD,"role":"writer"}).to_string(),
            Some("application/json"),
        )
        .await;
        assert_eq!(create_user.status(), StatusCode::CREATED);
        let role: String = sqlx::query_scalar("SELECT g.role FROM latex_core.global_user_roles g JOIN latex_core.user_credentials c ON c.user_id=g.user_id WHERE c.email=$1")
            .bind(&email).fetch_one(&pool).await.unwrap();
        assert_eq!(role, "writer");
        let compatibility: String = sqlx::query_scalar(
            "SELECT account_type FROM latex_core.user_credentials WHERE email=$1",
        )
        .bind(&email)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(compatibility, "student");
        assert_eq!(
            request(
                &app,
                Method::POST,
                "/api/admin/v2/users",
                Some(&admin.cookie),
                &serde_json::json!({"email":format!("{}@s2.example", uuid::Uuid::new_v4()),"password":PASSWORD,"role":"mentor"}).to_string(),
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CREATED
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                "/api/admin/v2/users",
                Some(&admin.cookie),
                &serde_json::json!({"email":email,"password":PASSWORD,"role":"mentor"}).to_string(),
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            request(&app, Method::POST, "/api/admin/v2/users", Some(&admin.cookie),
                &serde_json::json!({"email":format!("{}@s2.example", uuid::Uuid::new_v4()),"password":PASSWORD,"role":"project_manager"}).to_string(), Some("application/json")).await.status(),
            StatusCode::BAD_REQUEST
        );
        for denied in [&writer, &mentor] {
            assert_eq!(request(&app, Method::POST, "/api/admin/v2/users", Some(&denied.cookie),
                &serde_json::json!({"email":format!("{}@s2.example", uuid::Uuid::new_v4()),"password":PASSWORD,"role":"writer"}).to_string(), Some("application/json")).await.status(), StatusCode::FORBIDDEN);
        }

        let writer_id = test_user_id(&pool, &writer.email).await;
        let other_writer_id = test_user_id(&pool, &other_writer.email).await;
        let mentor_id = test_user_id(&pool, &mentor.email).await;
        let admin_id = test_user_id(&pool, &admin.email).await;
        let unassigned_writer_id = test_user_id(&pool, &email).await;
        let team_response = request(
            &app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie),
            &serde_json::json!({"name":"S2 Team","writer_ids":[writer_id,other_writer_id],"leader_writer_id":writer_id,"mentor_ids":[mentor_id]}).to_string(),
            Some("application/json"),
        ).await;
        assert_eq!(team_response.status(), StatusCode::CREATED);
        let team = test_json(team_response).await;
        let team_id = team["team"]["id"].as_str().unwrap().to_owned();
        assert_eq!(team["main_file"]["path"], "main.tex");
        for invalid_leader in [mentor_id, admin_id, unassigned_writer_id] {
            assert_eq!(
                request(
                    &app,
                    Method::PATCH,
                    &format!("/api/admin/v2/paper-teams/{team_id}/leader"),
                    Some(&admin.cookie),
                    &serde_json::json!({"user_id":invalid_leader}).to_string(),
                    Some("application/json")
                )
                .await
                .status(),
                StatusCode::FORBIDDEN
            );
        }
        let changed_leader = test_json(
            request(
                &app,
                Method::PATCH,
                &format!("/api/admin/v2/paper-teams/{team_id}/leader"),
                Some(&admin.cookie),
                &serde_json::json!({"user_id":other_writer_id}).to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(changed_leader["user_id"], other_writer_id.to_string());
        assert_eq!(changed_leader["is_leader"], true);
        let team_detail = test_json(
            get(
                &app,
                &format!("/api/admin/v2/paper-teams/{team_id}"),
                Some(&admin.cookie),
            )
            .await,
        )
        .await;
        assert_eq!(
            team_detail["members"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|member| member["is_leader"] == true)
                .count(),
            1
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                "/api/admin/v2/paper-teams",
                Some(&admin.cookie),
                &serde_json::json!({"name":"Wrong role","writer_ids":[admin_id],"leader_writer_id":admin_id,"mentor_ids":[]})
                    .to_string(),
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        for denied in [&writer, &mentor] {
            assert_eq!(
                request(
                    &app,
                    Method::POST,
                    "/api/admin/v2/paper-teams",
                    Some(&denied.cookie),
                    r#"{"name":"Denied","writer_ids":[],"leader_writer_id":"","mentor_ids":[]}"#,
                    Some("application/json")
                )
                .await
                .status(),
                StatusCode::FORBIDDEN
            );
        }

        for denied in [&mentor, &admin] {
            assert_eq!(
                request(
                    &app,
                    Method::POST,
                    "/api/v2/writer/personal-papers",
                    Some(&denied.cookie),
                    r#"{"name":"Denied"}"#,
                    Some("application/json")
                )
                .await
                .status(),
                StatusCode::FORBIDDEN
            );
        }
        let personal = request(
            &app,
            Method::POST,
            "/api/v2/writer/personal-papers",
            Some(&writer.cookie),
            r#"{"name":"S2 Personal"}"#,
            Some("application/json"),
        )
        .await;
        assert_eq!(personal.status(), StatusCode::CREATED);
        let personal = test_json(personal).await;
        let paper_id = personal["paper"]["id"].as_str().unwrap().to_owned();
        assert_eq!(personal["main_file"]["path"], "main.tex");
        let visible =
            test_json(get(&app, "/api/v2/writer/papers", Some(&writer.cookie)).await).await;
        assert!(
            visible
                .as_array()
                .unwrap()
                .iter()
                .any(|paper| paper["id"] == paper_id)
        );
        assert!(
            visible
                .as_array()
                .unwrap()
                .iter()
                .any(|paper| paper["id"] == team_id)
        );
        assert_eq!(
            get(
                &app,
                &format!("/api/v2/papers/{paper_id}"),
                Some(&other_writer.cookie)
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );

        let files = test_json(
            get(
                &app,
                &format!("/api/v2/papers/{paper_id}/files"),
                Some(&writer.cookie),
            )
            .await,
        )
        .await;
        let main_id = files[0]["file_id"].as_str().unwrap().to_owned();
        let main_path = format!("/api/v2/papers/{paper_id}/files/{main_id}");
        let saved = request(
            &app,
            Method::PUT,
            &main_path,
            Some(&writer.cookie),
            r#"{"content":"newer source","version":1}"#,
            Some("application/json"),
        )
        .await;
        assert_eq!(test_json(saved).await["version"], 2);
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &main_path,
                Some(&writer.cookie),
                r#"{"content":"stale source","version":1}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            test_json(get(&app, &main_path, Some(&writer.cookie)).await).await["content"],
            "newer source"
        );

        let created = request(
            &app,
            Method::POST,
            &format!("/api/v2/papers/{paper_id}/files"),
            Some(&writer.cookie),
            r#"{"path":"chapters/one.tex","content":"chapter","version":2}"#,
            Some("application/json"),
        )
        .await;
        let created = test_json(created).await;
        let second_id = created["file"]["file_id"].as_str().unwrap().to_owned();
        let renamed = request(
            &app,
            Method::PATCH,
            &format!("/api/v2/papers/{paper_id}/files/{second_id}/path"),
            Some(&writer.cookie),
            r#"{"path":"sections/one.tex","version":3}"#,
            Some("application/json"),
        )
        .await;
        let renamed = test_json(renamed).await;
        assert_eq!(renamed["file"]["file_id"], second_id);
        assert_eq!(renamed["file"]["path"], "sections/one.tex");
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/papers/{paper_id}/main/{second_id}"),
                Some(&writer.cookie),
                r#"{"version":4}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(
                &app,
                Method::DELETE,
                &format!("/api/v2/papers/{paper_id}/files/{second_id}"),
                Some(&writer.cookie),
                r#"{"version":5}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert!(
            sqlx::query_scalar::<_, bool>(
                "SELECT tombstoned FROM latex_core.paper_files WHERE file_id=$1"
            )
            .bind(uuid::Uuid::parse_str(&second_id).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap()
        );
        for denied in [&mentor, &admin] {
            assert_eq!(
                request(
                    &app,
                    Method::PUT,
                    &main_path,
                    Some(&denied.cookie),
                    r#"{"content":"denied","version":6}"#,
                    Some("application/json")
                )
                .await
                .status(),
                StatusCode::FORBIDDEN
            );
        }

        let login = login_request(&app, &email, PASSWORD).await;
        let old_cookie = response_cookie(&login);
        let provisioned_id = test_user_id(&pool, &email).await;
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &format!("/api/admin/v2/users/{provisioned_id}/role"),
                Some(&admin.cookie),
                r#"{"role":"mentor"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            get(&app, "/api/v2/me", Some(&old_cookie)).await.status(),
            StatusCode::UNAUTHORIZED
        );

        pool.close().await;
        database.close().await;
    }

    #[tokio::test]
    async fn admin_browser_template_zip_preview_import_and_validation() {
        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, _state) = test_application().await;
        let admin = fixture(&app, &database, "student", Some(GlobalRole::Admin)).await;
        let mentor = fixture(&app, &database, "student", Some(GlobalRole::Mentor)).await;
        let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let writer_id = test_user_id(&pool, &writer.email).await;
        let mentor_id = test_user_id(&pool, &mentor.email).await;
        let archive = test_zip(&[
            (
                "main.tex",
                b"\\documentclass{article}\\begin{document}Imported\\end{document}",
            ),
            ("sections/intro.tex", b"Introduction"),
            ("assets/data.csv", b"x,y\n1,2\n"),
        ]);
        let (preview_body, preview_type) = template_multipart(&[], &archive);
        let preview = request_bytes(
            &app,
            Method::POST,
            "/api/admin/v2/templates/preview",
            Some(&admin.cookie),
            preview_body.clone(),
            Some(&preview_type),
        )
        .await;
        assert_eq!(preview.status(), StatusCode::OK);
        let preview = test_json(preview).await;
        assert_eq!(preview["detected_main"], "main.tex");
        assert_eq!(preview["files"].as_array().unwrap().len(), 3);
        assert_eq!(
            request_bytes(
                &app,
                Method::POST,
                "/api/admin/v2/templates/preview",
                Some(&mentor.cookie),
                preview_body,
                Some(&preview_type),
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let (import_body, import_type) = template_multipart(
            &[
                ("name", "Browser Template"),
                ("description", "Local ZIP"),
                ("main", "main.tex"),
            ],
            &archive,
        );
        let imported = request_bytes(
            &app,
            Method::POST,
            "/api/admin/v2/templates/import",
            Some(&admin.cookie),
            import_body,
            Some(&import_type),
        )
        .await;
        assert_eq!(imported.status(), StatusCode::CREATED);
        let imported = test_json(imported).await;
        assert_eq!(imported["main_file"], "main.tex");
        assert_eq!(imported["files"].as_array().unwrap().len(), 3);
        assert_eq!(imported["source_identity"].as_str().unwrap().len(), 64);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM latex_core.template_files WHERE template_id=$1",
            )
            .bind(uuid::Uuid::parse_str(imported["id"].as_str().unwrap()).unwrap())
            .fetch_one(&pool)
            .await
            .unwrap(),
            3
        );
        let template_id = imported["id"].as_str().unwrap();
        let listed = test_json(get(&app, "/api/admin/templates", Some(&admin.cookie)).await).await;
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert_eq!(listed[0]["name"], "Browser Template");
        assert_eq!(listed[0]["pinned"], false);
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &format!("/api/admin/v2/templates/{template_id}"),
                Some(&admin.cookie),
                r#"{"name":"Edited Template","description":"Edited","main_file":"sections/intro.tex"}"#,
                Some("application/json"),
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        let created = test_json(
            request(
                &app,
                Method::POST,
                "/api/admin/v2/paper-teams",
                Some(&admin.cookie),
                &serde_json::json!({"name":"Imported Template Team","writer_ids":[writer_id],"leader_writer_id":writer_id,"mentor_ids":[mentor_id],"template_id":template_id}).to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(created["template_pin"]["template_id"], template_id);
        let paper_id = created["team"]["id"].as_str().unwrap();
        let detail = test_json(
            get(
                &app,
                &format!("/api/admin/v2/paper-teams/{paper_id}"),
                Some(&admin.cookie),
            )
            .await,
        )
        .await;
        assert_eq!(detail["template_pin"]["template_name"], "Edited Template");
        let pinned_remove = request(
            &app,
            Method::DELETE,
            &format!("/api/admin/v2/templates/{template_id}"),
            Some(&admin.cookie),
            "",
            None,
        )
        .await;
        assert_eq!(pinned_remove.status(), StatusCode::CONFLICT);
        assert_eq!(
            test_json(pinned_remove).await["error"],
            "Template is in use by a Paper Team and cannot be removed."
        );

        let unused_archive = test_zip(&[("main.tex", b"unused")]);
        let (unused_body, unused_type) = template_multipart(
            &[("name", "Unused Template"), ("main", "main.tex")],
            &unused_archive,
        );
        let unused = test_json(
            request_bytes(
                &app,
                Method::POST,
                "/api/admin/v2/templates/import",
                Some(&admin.cookie),
                unused_body,
                Some(&unused_type),
            )
            .await,
        )
        .await;
        assert_eq!(
            request(
                &app,
                Method::DELETE,
                &format!("/api/admin/v2/templates/{}", unused["id"].as_str().unwrap()),
                Some(&admin.cookie),
                "",
                None,
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );

        let ambiguous = test_zip(&[("a.tex", b"a"), ("b.tex", b"b")]);
        let (ambiguous, ambiguous_type) = template_multipart(&[("name", "Ambiguous")], &ambiguous);
        assert_eq!(
            request_bytes(
                &app,
                Method::POST,
                "/api/admin/v2/templates/import",
                Some(&admin.cookie),
                ambiguous,
                Some(&ambiguous_type),
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        let no_tex = test_zip(&[("README.md", b"not TeX")]);
        let (no_tex, no_tex_type) = template_multipart(&[], &no_tex);
        assert_eq!(
            request_bytes(
                &app,
                Method::POST,
                "/api/admin/v2/templates/preview",
                Some(&admin.cookie),
                no_tex,
                Some(&no_tex_type),
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        let traversal = test_zip(&[("../escape.tex", b"unsafe")]);
        let (traversal, traversal_type) = template_multipart(&[], &traversal);
        assert_eq!(
            request_bytes(
                &app,
                Method::POST,
                "/api/admin/v2/templates/preview",
                Some(&admin.cookie),
                traversal,
                Some(&traversal_type),
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        pool.close().await;
        database.close().await;
    }

    fn test_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut output);
            for (path, bytes) in entries {
                writer
                    .start_file(*path, zip::write::SimpleFileOptions::default())
                    .unwrap();
                writer.write_all(bytes).unwrap();
            }
            writer.finish().unwrap();
        }
        output.into_inner()
    }

    fn template_multipart(fields: &[(&str, &str)], archive: &[u8]) -> (Vec<u8>, String) {
        const BOUNDARY: &str = "latex-core-template-test-boundary";
        let mut body = Vec::new();
        for (name, value) in fields {
            body.extend_from_slice(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
        }
        body.extend_from_slice(format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"archive\"; filename=\"template.zip\"\r\nContent-Type: application/zip\r\n\r\n").as_bytes());
        body.extend_from_slice(archive);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
        (body, format!("multipart/form-data; boundary={BOUNDARY}"))
    }

    async fn test_json(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn test_user_id(pool: &PgPool, email: &str) -> UserId {
        UserId::from_uuid(
            sqlx::query_scalar("SELECT user_id FROM latex_core.user_credentials WHERE email=$1")
                .bind(email)
                .fetch_one(pool)
                .await
                .unwrap(),
        )
    }

    #[tokio::test]
    async fn s5_review_authorization_lifecycle_suggestions_rounds_and_isolation() {
        use core_types::WorkerId;

        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, state) = test_application().await;
        let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
        let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let other_writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let other_mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let writer_id = test_user_id(&pool, &writer.email).await;
        let other_writer_id = test_user_id(&pool, &other_writer.email).await;
        let mentor_id = test_user_id(&pool, &mentor.email).await;
        let empty_admin_reviews = get(&app, "/api/admin/v2/reviews", Some(&admin.cookie)).await;
        assert_eq!(empty_admin_reviews.status(), StatusCode::OK);
        assert!(
            test_json(empty_admin_reviews)
                .await
                .as_array()
                .unwrap()
                .is_empty()
        );

        let created = test_json(
            request(
                &app,
                Method::POST,
                "/api/admin/v2/paper-teams",
                Some(&admin.cookie),
                &serde_json::json!({"name":"S5 Review Team","writer_ids":[writer_id,other_writer_id],"leader_writer_id":writer_id,"mentor_ids":[mentor_id]}).to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let paper_id = created["team"]["id"].as_str().unwrap();
        let workspace_id =
            uuid::Uuid::parse_str(created["team"]["workspace_id"].as_str().unwrap()).unwrap();
        let file_id =
            uuid::Uuid::parse_str(created["main_file"]["file_id"].as_str().unwrap()).unwrap();
        let review_root = format!("/api/v2/reviews/papers/{paper_id}");
        let source_anchor = serde_json::json!({
            "file_id":file_id,"encoded_relative_start":[1],"encoded_relative_end":[2],
            "quoted_text":"article","context_hash":"0".repeat(64),"source_sequence":1,
            "source_version_id":null,"document_epoch":1
        });
        let comment = serde_json::json!({
            "thread_type":"COMMENT","message":"Clarify this paragraph",
            "source_anchor":source_anchor,"pdf_anchor":null
        });

        let listed =
            test_json(get(&app, "/api/v2/mentor/papers", Some(&mentor.cookie)).await).await;
        assert_eq!(listed["papers"][0]["id"], paper_id);
        assert!(
            listed["papers"][0]["pdf_available"]
                .as_bool()
                .is_some_and(|value| !value)
        );
        assert!(test_json(get(&app, "/api/v2/mentor/papers", Some(&other_mentor.cookie)).await).await["papers"].as_array().unwrap().is_empty());
        assert_eq!(
            get(&app, "/api/v2/mentor/papers", Some(&admin.cookie))
                .await
                .status(),
            StatusCode::FORBIDDEN
        );

        let before_review = request(
            &app,
            Method::POST,
            &format!("{review_root}/threads"),
            Some(&mentor.cookie),
            &comment.to_string(),
            Some("application/json"),
        )
        .await;
        assert_eq!(before_review.status(), StatusCode::CONFLICT);
        assert_eq!(
            test_json(before_review).await["error"],
            "This paper has not been sent for review."
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/rounds"),
                Some(&other_writer.cookie),
                "{}",
                Some("application/json"),
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );

        let missing_baseline = request(
            &app,
            Method::POST,
            &format!("{review_root}/rounds"),
            Some(&writer.cookie),
            "{}",
            Some("application/json"),
        )
        .await;
        assert_eq!(missing_baseline.status(), StatusCode::CONFLICT);
        assert_eq!(
            test_json(missing_baseline).await["error"],
            "Compile the current paper before sending it for review."
        );

        let build_path = format!("/api/v2/papers/{paper_id}/builds");
        assert_eq!(
            request(
                &app,
                Method::POST,
                &build_path,
                Some(&mentor.cookie),
                r#"{"trigger_type":"auto"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let build = request(
            &app,
            Method::POST,
            &build_path,
            Some(&mentor.cookie),
            r#"{"trigger_type":"manual"}"#,
            Some("application/json"),
        )
        .await;
        assert_eq!(build.status(), StatusCode::ACCEPTED);
        let worker = WorkerId::new();
        let claimed = state.queue.claim(worker).await.unwrap().unwrap();
        state
            .queue
            .complete_success(
                claimed.id,
                worker,
                &test_artifacts(&state, "s5").await,
                core_types::BlobHash::digest(b"s5-manifest"),
            )
            .await
            .unwrap();
        let current = test_json(get(&app, &build_path, Some(&mentor.cookie)).await).await;
        assert!(current["build"]["current_build_id"].is_string());

        let legacy_round_id = uuid::Uuid::new_v4();
        let legacy_thread_id = uuid::Uuid::new_v4();
        let baseline_version_id: uuid::Uuid = sqlx::query_scalar(
            "SELECT id FROM latex_core.paper_versions WHERE workspace_id=$1 ORDER BY version_number DESC LIMIT 1",
        )
        .bind(workspace_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO latex_core.review_rounds \
             (id,paper_id,workspace_id,round_number,baseline_version_id,opened_by_mentor_user_id,status) \
             VALUES ($1,$2,$3,1,$4,$5,'OPEN')",
        )
        .bind(legacy_round_id)
        .bind(uuid::Uuid::parse_str(paper_id).unwrap())
        .bind(workspace_id)
        .bind(baseline_version_id)
        .bind(mentor_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO latex_core.review_threads \
             (id,review_round_id,workspace_id,thread_type,severity,category,created_by_mentor_user_id) \
             VALUES ($1,$2,$3,'COMMENT','NOTE','WRITING',$4)",
        )
        .bind(legacy_thread_id)
        .bind(legacy_round_id)
        .bind(workspace_id)
        .bind(mentor_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO latex_core.review_messages (id,thread_id,author_user_id,body) \
             VALUES ($1,$2,$3,'Historical review comment')",
        )
        .bind(uuid::Uuid::new_v4())
        .bind(legacy_thread_id)
        .bind(mentor_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();

        let legacy_state =
            test_json(get(&app, &format!("{review_root}/rounds"), Some(&mentor.cookie)).await)
                .await;
        assert_eq!(legacy_state["review_open"], false);
        assert!(legacy_state["current_review_round"].is_null());
        assert_eq!(legacy_state["rounds"][0]["status"], "OPEN");

        let round = request(
            &app,
            Method::POST,
            &format!("{review_root}/rounds"),
            Some(&writer.cookie),
            "{}",
            Some("application/json"),
        )
        .await;
        assert_eq!(round.status(), StatusCode::CREATED);
        let round = test_json(round).await;
        let round_id = round["id"].as_str().unwrap();
        assert_eq!(round["round_number"], 2);
        assert_eq!(round["status"], "OPEN_FOR_REVIEW");
        assert_eq!(
            round["submitted_by_leader_writer_id"],
            writer_id.to_string()
        );
        assert!(round["baseline_version_id"].is_string());
        assert!(round["baseline_build_id"].is_string());
        let first_baseline_hash = round["baseline_state_hash"].as_str().unwrap().to_owned();
        let legacy_after: (String, bool, i64) = sqlx::query_as(
            "SELECT rr.status,rr.closed_at IS NOT NULL,count(rm.id) \
             FROM latex_core.review_rounds rr \
             LEFT JOIN latex_core.review_threads rt ON rt.review_round_id=rr.id \
             LEFT JOIN latex_core.review_messages rm ON rm.thread_id=rt.id \
             WHERE rr.id=$1 GROUP BY rr.id",
        )
        .bind(legacy_round_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(legacy_after.0, "CLOSED");
        assert!(legacy_after.1);
        assert_eq!(legacy_after.2, 1, "legacy comments must be retained");

        let repeated = request(
            &app,
            Method::POST,
            &format!("{review_root}/rounds"),
            Some(&writer.cookie),
            "{}",
            Some("application/json"),
        )
        .await;
        assert_eq!(repeated.status(), StatusCode::OK);
        assert_eq!(test_json(repeated).await["id"], round_id);
        for participant in [&writer, &mentor] {
            let state = test_json(
                get(
                    &app,
                    &format!("{review_root}/rounds"),
                    Some(&participant.cookie),
                )
                .await,
            )
            .await;
            assert_eq!(state["review_open"], true);
            assert_eq!(state["current_review_round"]["id"], round_id);
        }
        for denied in [&writer, &admin] {
            assert_eq!(
                request(
                    &app,
                    Method::POST,
                    &format!("{review_root}/threads"),
                    Some(&denied.cookie),
                    &comment.to_string(),
                    Some("application/json")
                )
                .await
                .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/threads"),
                Some(&other_mentor.cookie),
                &comment.to_string(),
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        let created_thread = request(
            &app,
            Method::POST,
            &format!("{review_root}/threads"),
            Some(&mentor.cookie),
            &comment.to_string(),
            Some("application/json"),
        )
        .await;
        assert_eq!(created_thread.status(), StatusCode::CREATED);
        let thread_id = test_json(created_thread).await["thread_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let admin_reviews =
            test_json(get(&app, "/api/admin/v2/reviews", Some(&admin.cookie)).await).await;
        assert_eq!(
            admin_reviews.as_array().unwrap().len(),
            2,
            "current and historical review comments remain visible to Admin"
        );
        assert_eq!(admin_reviews[0]["paper_name"], "S5 Review Team");
        assert_eq!(admin_reviews[0]["thread_type"], "COMMENT");
        assert_eq!(admin_reviews[0]["severity"], "NOTE");
        assert_eq!(admin_reviews[0]["category"], "WRITING");
        assert_eq!(admin_reviews[0]["mentor"], mentor.email);
        assert!(admin_reviews[0]["assigned_writer"].is_null());
        assert_eq!(
            get(&app, &format!("{review_root}/threads"), Some(&admin.cookie),)
                .await
                .status(),
            StatusCode::FORBIDDEN,
            "Admin review access remains inspection-only",
        );

        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/threads/{thread_id}/messages"),
                Some(&writer.cookie),
                r#"{"body":"Writer reply"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CREATED
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/threads/{thread_id}/state"),
                Some(&writer.cookie),
                r#"{"state":"RESOLVED"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );

        let before_source = test_json(
            get(
                &app,
                &format!("{review_root}/files/{file_id}"),
                Some(&mentor.cookie),
            )
            .await,
        )
        .await["content"]
            .as_str()
            .unwrap()
            .to_owned();
        let suggestion = serde_json::json!({"thread_type":"SUGGESTION","message":"replacement","source_anchor":source_anchor,"pdf_anchor":null});
        let suggested = test_json(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/threads"),
                Some(&mentor.cookie),
                &suggestion.to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let suggestion_id = suggested["thread_id"].as_str().unwrap();
        let after_mentor = test_json(
            get(
                &app,
                &format!("{review_root}/files/{file_id}"),
                Some(&mentor.cookie),
            )
            .await,
        )
        .await["content"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            before_source, after_mentor,
            "Mentor suggestion must not mutate source"
        );
        let durable_sequence: i64 = sqlx::query_scalar(
            "INSERT INTO latex_core.collaboration_updates (workspace_id,file_id,document_epoch,actor_user_id,update_bytes) VALUES ($1,$2,1,$3,$4) RETURNING id",
        ).bind(workspace_id).bind(file_id).bind(writer_id.as_uuid()).bind(vec![1_u8]).fetch_one(&pool).await.unwrap();
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/threads/{suggestion_id}/suggestion/accept"),
                Some(&writer.cookie),
                &serde_json::json!({"durable_sequence":durable_sequence}).to_string(),
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );
        let accepted: (String, uuid::Uuid) = sqlx::query_as("SELECT status,accepted_by_writer_user_id FROM latex_core.review_suggestions WHERE thread_id=$1").bind(uuid::Uuid::parse_str(suggestion_id).unwrap()).fetch_one(&pool).await.unwrap();
        assert_eq!(accepted.0, "ACCEPTED");
        assert_eq!(accepted.1, *writer_id.as_uuid());

        let rejected = test_json(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/threads"),
                Some(&mentor.cookie),
                &suggestion.to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!(
                    "{review_root}/threads/{}/suggestion/reject",
                    rejected["thread_id"].as_str().unwrap()
                ),
                Some(&writer.cookie),
                r#"{"rejection_reason":"Not appropriate"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::NO_CONTENT
        );

        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/rounds/{round_id}/approve"),
                Some(&mentor.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::GONE
        );
        let closed = request(
            &app,
            Method::POST,
            &format!("{review_root}/rounds/{round_id}/close"),
            Some(&writer.cookie),
            "{}",
            Some("application/json"),
        )
        .await;
        assert_eq!(closed.status(), StatusCode::OK);
        assert_eq!(test_json(closed).await["status"], "CLOSED");
        let closed_state =
            test_json(get(&app, &format!("{review_root}/rounds"), Some(&mentor.cookie)).await)
                .await;
        assert_eq!(closed_state["review_open"], false);
        assert!(closed_state["current_review_round"].is_null());
        let after_close = request(
            &app,
            Method::POST,
            &format!("{review_root}/threads"),
            Some(&mentor.cookie),
            &comment.to_string(),
            Some("application/json"),
        )
        .await;
        assert_eq!(after_close.status(), StatusCode::CONFLICT);
        assert_eq!(
            test_json(after_close).await["error"],
            "This paper has not been sent for review."
        );

        let paper_detail = test_json(
            get(
                &app,
                &format!("/api/v2/papers/{paper_id}"),
                Some(&writer.cookie),
            )
            .await,
        )
        .await;
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &format!("/api/v2/papers/{paper_id}/files/{file_id}"),
                Some(&writer.cookie),
                &serde_json::json!({"content":"\\documentclass{article}\n\\begin{document}newer source\\end{document}","version":paper_detail["version"]}).to_string(),
                Some("application/json"),
            )
            .await
            .status(),
            StatusCode::OK
        );
        let newer = state
            .workspaces
            .force_snapshot(WorkspaceId::from_uuid(workspace_id))
            .await
            .unwrap();
        assert_ne!(first_baseline_hash, newer.snapshot_id().to_hex());
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/rounds"),
                Some(&writer.cookie),
                "{}",
                Some("application/json"),
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &build_path,
                Some(&writer.cookie),
                r#"{"trigger_type":"manual"}"#,
                Some("application/json"),
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        let second_worker = WorkerId::new();
        let second_claimed = state.queue.claim(second_worker).await.unwrap().unwrap();
        state
            .queue
            .complete_success(
                second_claimed.id,
                second_worker,
                &test_artifacts(&state, "s5-second").await,
                core_types::BlobHash::digest(b"s5-second-manifest"),
            )
            .await
            .unwrap();
        let second_round = request(
            &app,
            Method::POST,
            &format!("{review_root}/rounds"),
            Some(&writer.cookie),
            "{}",
            Some("application/json"),
        )
        .await;
        assert_eq!(second_round.status(), StatusCode::CREATED);
        let second_round = test_json(second_round).await;
        assert_eq!(second_round["round_number"], 3);
        assert_eq!(second_round["status"], "OPEN_FOR_REVIEW");
        assert_ne!(second_round["baseline_state_hash"], first_baseline_hash);
        let reopened_state =
            test_json(get(&app, &format!("{review_root}/rounds"), Some(&mentor.cookie)).await)
                .await;
        assert_eq!(reopened_state["review_open"], true);
        assert_eq!(
            reopened_state["current_review_round"]["id"],
            second_round["id"]
        );

        assert_eq!(
            get(
                &app,
                &format!("{review_root}/threads"),
                Some(&other_writer.cookie)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            get(
                &app,
                &format!("{review_root}/threads"),
                Some(&other_mentor.cookie)
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get(
                &app,
                &format!("{review_root}/report.csv"),
                Some(&mentor.cookie)
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            get(
                &app,
                &format!("{review_root}/report.html"),
                Some(&writer.cookie)
            )
            .await
            .status(),
            StatusCode::OK
        );
        sqlx::query("UPDATE latex_core.paper_files SET tombstoned=true,tombstoned_at=statement_timestamp() WHERE file_id=$1")
            .bind(file_id)
            .execute(&pool)
            .await
            .unwrap();
        let after_delete = test_json(
            get(
                &app,
                &format!("{review_root}/threads"),
                Some(&mentor.cookie),
            )
            .await,
        )
        .await;
        assert!(
            after_delete["threads"]
                .as_array()
                .unwrap()
                .iter()
                .any(|thread| { thread["source_anchor"]["file_deleted"] == true })
        );

        pool.close().await;
        database.close().await;
    }

    #[tokio::test]
    async fn realtime_collaboration_converges_persists_recovers_and_enforces_access() {
        use futures_util::SinkExt;
        use persistence::CollaborationAccessMode;
        use yrs::{Doc, GetString, ReadTxn, Text, Transact, Update, updates::decoder::Decode};

        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, state) = test_application().await;
        let writer_a = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let writer_b = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let outsider = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
        let writer_a_id = test_user_id(&pool, &writer_a.email).await;
        let writer_b_id = test_user_id(&pool, &writer_b.email).await;
        let mentor_id = test_user_id(&pool, &mentor.email).await;
        let outsider_id = test_user_id(&pool, &outsider.email).await;
        let admin_id = test_user_id(&pool, &admin.email).await;

        let created = request(
            &app,
            Method::POST,
            "/api/admin/v2/paper-teams",
            Some(&admin.cookie),
            &serde_json::json!({
                "name":"S3 realtime",
                "writer_ids":[writer_a_id,writer_b_id],
                "leader_writer_id":writer_a_id,
                "mentor_ids":[mentor_id]
            })
            .to_string(),
            Some("application/json"),
        )
        .await;
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = test_json(created).await;
        let paper_id = uuid::Uuid::parse_str(created["team"]["id"].as_str().unwrap()).unwrap();
        let workspace_id = WorkspaceId::from_uuid(
            uuid::Uuid::parse_str(created["team"]["workspace_id"].as_str().unwrap()).unwrap(),
        );
        let file_id =
            uuid::Uuid::parse_str(created["main_file"]["file_id"].as_str().unwrap()).unwrap();
        let versions_before_save: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.paper_versions WHERE workspace_id=$1",
        )
        .bind(workspace_id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(
            state
                .v2
                .collaboration_access(writer_a_id, paper_id, file_id)
                .await
                .unwrap()
                .mode,
            CollaborationAccessMode::ReadWrite
        );
        assert_eq!(
            state
                .v2
                .collaboration_access(writer_b_id, paper_id, file_id)
                .await
                .unwrap()
                .mode,
            CollaborationAccessMode::ReadWrite
        );
        assert_eq!(
            state
                .v2
                .collaboration_access(mentor_id, paper_id, file_id)
                .await
                .unwrap()
                .mode,
            CollaborationAccessMode::ReadOnly
        );
        assert!(
            state
                .v2
                .collaboration_access(outsider_id, paper_id, file_id)
                .await
                .is_err()
        );
        assert!(
            state
                .v2
                .collaboration_access(admin_id, paper_id, file_id)
                .await
                .is_err()
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let route = format!("ws://{address}/api/v2/collab/{paper_id}/files/{file_id}");
        let (mut socket_a, initial_a, access_a) = ws_test_join(&route, &writer_a.cookie).await;
        let (mut socket_b, initial_b, access_b) = ws_test_join(&route, &writer_b.cookie).await;
        let (mut socket_m, initial_m, access_m) = ws_test_join(&route, &mentor.cookie).await;
        assert_eq!(access_a, "read_write");
        assert_eq!(access_b, "read_write");
        assert_eq!(access_m, "read_only");

        let doc_a = Doc::new();
        let text_a = doc_a.get_or_insert_text("source");
        doc_a
            .transact_mut()
            .apply_update(Update::decode_v1(&initial_a).unwrap())
            .unwrap();
        let doc_b = Doc::new();
        let text_b = doc_b.get_or_insert_text("source");
        doc_b
            .transact_mut()
            .apply_update(Update::decode_v1(&initial_b).unwrap())
            .unwrap();
        let doc_m = Doc::new();
        let text_m = doc_m.get_or_insert_text("source");
        doc_m
            .transact_mut()
            .apply_update(Update::decode_v1(&initial_m).unwrap())
            .unwrap();

        let before_a = doc_a.transact().state_vector();
        let end_a = text_a.len(&doc_a.transact());
        text_a.insert(&mut doc_a.transact_mut(), end_a, " Writer-A");
        let update_a = doc_a.transact().encode_diff_v1(&before_a);
        socket_a.send(ws_source_update(1, &update_a)).await.unwrap();
        let remote_for_b = ws_remote_update(&mut socket_b).await;
        doc_b
            .transact_mut()
            .apply_update(Update::decode_v1(&remote_for_b).unwrap())
            .unwrap();
        let remote_for_m = ws_remote_update(&mut socket_m).await;
        doc_m
            .transact_mut()
            .apply_update(Update::decode_v1(&remote_for_m).unwrap())
            .unwrap();
        ws_durable_ack(&mut socket_a, 1).await;

        let before_b = doc_b.transact().state_vector();
        let end_b = text_b.len(&doc_b.transact());
        text_b.insert(&mut doc_b.transact_mut(), end_b, " Writer-B");
        let update_b = doc_b.transact().encode_diff_v1(&before_b);
        socket_b.send(ws_source_update(2, &update_b)).await.unwrap();
        let remote_for_a = ws_remote_update(&mut socket_a).await;
        doc_a
            .transact_mut()
            .apply_update(Update::decode_v1(&remote_for_a).unwrap())
            .unwrap();
        let remote_for_m = ws_remote_update(&mut socket_m).await;
        doc_m
            .transact_mut()
            .apply_update(Update::decode_v1(&remote_for_m).unwrap())
            .unwrap();
        ws_durable_ack(&mut socket_b, 2).await;
        assert_eq!(
            text_a.get_string(&doc_a.transact()),
            text_b.get_string(&doc_b.transact())
        );
        assert_eq!(
            text_a.get_string(&doc_a.transact()),
            text_m.get_string(&doc_m.transact())
        );

        let durable_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.collaboration_updates \
             WHERE workspace_id=$1 AND file_id=$2",
        )
        .bind(workspace_id.as_uuid())
        .bind(file_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(durable_count, 2);
        let versions_after_save: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.paper_versions WHERE workspace_id=$1",
        )
        .bind(workspace_id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(versions_after_save, versions_before_save);

        socket_m.send(ws_source_update(9, &update_b)).await.unwrap();
        let mentor_error = ws_control(&mut socket_m, "ERROR").await;
        assert_eq!(mentor_error["code"], "read_only");
        let after_rejection: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.collaboration_updates \
             WHERE workspace_id=$1 AND file_id=$2",
        )
        .bind(workspace_id.as_uuid())
        .bind(file_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(after_rejection, durable_count);

        let recovery = state
            .v2
            .collaboration_recovery(workspace_id, file_id, 1)
            .await
            .unwrap();
        let recovered = Doc::new();
        let recovered_text = recovered.get_or_insert_text("source");
        if let Some((_, compressed)) = recovery.snapshot {
            let bytes = zstd::stream::decode_all(Cursor::new(compressed)).unwrap();
            recovered
                .transact_mut()
                .apply_update(Update::decode_v1(&bytes).unwrap())
                .unwrap();
        }
        for (_, update) in recovery.updates {
            recovered
                .transact_mut()
                .apply_update(Update::decode_v1(&update).unwrap())
                .unwrap();
        }
        let converged = text_a.get_string(&doc_a.transact());
        assert_eq!(recovered_text.get_string(&recovered.transact()), converged);
        let canonical = state
            .workspaces
            .read_file(workspace_id, &LogicalPath::parse("main.tex").unwrap())
            .await
            .unwrap();
        assert_eq!(String::from_utf8(canonical.to_vec()).unwrap(), converged);

        let paper = state.v2.writer_paper(writer_a_id, paper_id).await.unwrap();
        let current = state.workspaces.restore(workspace_id).await.unwrap();
        let renamed = state
            .v2
            .rename_file_with_event(
                file_id,
                writer_a_id,
                current.version().get(),
                LogicalPath::parse("renamed.tex").unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(renamed.0.file_id, file_id);
        assert_eq!(paper.workspace_id, renamed.0.workspace_id);
        assert_eq!(
            state
                .v2
                .collaboration_access(writer_a_id, paper_id, file_id)
                .await
                .unwrap()
                .file
                .path
                .as_str(),
            "renamed.tex"
        );

        socket_a.close(None).await.unwrap();
        socket_b.close(None).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let snapshots: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.collaboration_snapshots \
             WHERE workspace_id=$1 AND file_id=$2",
        )
        .bind(workspace_id.as_uuid())
        .bind(file_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(snapshots >= 1);

        server.abort();
        pool.close().await;
        database.close().await;
    }

    #[tokio::test]
    async fn s4_versions_coalesce_deduplicate_and_suppress_stale_promotion() {
        use core_types::WorkerId;

        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, state) = test_application().await;
        let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let outsider = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;

        let created = test_json(
            request(
                &app,
                Method::POST,
                "/api/v2/writer/personal-papers",
                Some(&writer.cookie),
                r#"{"name":"S4 exact paper"}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        let paper_id = created["paper"]["id"].as_str().unwrap();
        let workspace_id =
            uuid::Uuid::parse_str(created["paper"]["workspace_id"].as_str().unwrap()).unwrap();
        let file_id = created["main_file"]["file_id"].as_str().unwrap();
        let checkpoint_path = format!("/api/v2/papers/{paper_id}/versions");
        let build_path = format!("/api/v2/papers/{paper_id}/builds");
        let file_path = format!("/api/v2/papers/{paper_id}/files/{file_id}");

        let first_checkpoint = request(
            &app,
            Method::POST,
            &checkpoint_path,
            Some(&writer.cookie),
            r#"{"name":"Before edits"}"#,
            Some("application/json"),
        )
        .await;
        assert_eq!(first_checkpoint.status(), StatusCode::CREATED);
        let first_checkpoint = test_json(first_checkpoint).await;
        assert_eq!(first_checkpoint["name"], "Before edits");

        let h1 = test_json(
            request(
                &app,
                Method::POST,
                &build_path,
                Some(&writer.cookie),
                r#"{"trigger_type":"auto"}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        let h1_id = uuid::Uuid::parse_str(h1["build_id"].as_str().unwrap()).unwrap();
        let worker = WorkerId::new();
        let claimed_h1 = state.queue.claim(worker).await.unwrap().unwrap();
        assert_eq!(*claimed_h1.id.as_uuid(), h1_id);

        let mut version = 1_u64;
        let mut newest_hash = String::new();
        for label in ["H2", "H3", "H4"] {
            let saved = test_json(
                request(
                    &app,
                    Method::PUT,
                    &file_path,
                    Some(&writer.cookie),
                    &serde_json::json!({
                        "content":format!("\\documentclass{{article}}\n\\begin{{document}}{label}\\end{{document}}"),
                        "version":version
                    })
                    .to_string(),
                    Some("application/json"),
                )
                .await,
            )
            .await;
            version = saved["version"].as_u64().unwrap();
            let invalidated_pending: Option<String> = sqlx::query_scalar(
                "SELECT pending_state_hash FROM latex_core.v2_paper_build_state WHERE workspace_id=$1",
            )
            .bind(workspace_id)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert!(invalidated_pending.is_none());
            let submitted = test_json(
                request(
                    &app,
                    Method::POST,
                    &build_path,
                    Some(&writer.cookie),
                    r#"{"trigger_type":"auto"}"#,
                    Some("application/json"),
                )
                .await,
            )
            .await;
            assert_eq!(submitted["status"], "pending");
            newest_hash = submitted["state_hash"].as_str().unwrap().to_owned();
        }
        let effective: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.compile_jobs WHERE workspace_id=$1 AND state IN ('queued','running')",
        )
        .bind(workspace_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(effective, 1);
        let pending: String = sqlx::query_scalar(
            "SELECT pending_state_hash FROM latex_core.v2_paper_build_state WHERE workspace_id=$1",
        )
        .bind(workspace_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pending, newest_hash);

        let h1_artifacts = test_artifacts(&state, "h1").await;
        state
            .queue
            .complete_success(
                claimed_h1.id,
                worker,
                &h1_artifacts,
                core_types::BlobHash::digest(b"h1-manifest"),
            )
            .await
            .unwrap();
        let after_stale = test_json(get(&app, &build_path, Some(&writer.cookie)).await).await;
        assert!(after_stale["build"]["current_build_id"].is_null());
        let h4_id = after_stale["build"]["active_build_id"].as_str().unwrap();
        assert_ne!(h4_id, h1_id.to_string());

        let worker_h4 = WorkerId::new();
        let claimed_h4 = state.queue.claim(worker_h4).await.unwrap().unwrap();
        assert_eq!(claimed_h4.id.to_string(), h4_id);
        let h4_artifacts = test_artifacts(&state, "h4").await;
        state
            .queue
            .complete_success(
                claimed_h4.id,
                worker_h4,
                &h4_artifacts,
                core_types::BlobHash::digest(b"h4-manifest"),
            )
            .await
            .unwrap();
        let current = test_json(get(&app, &build_path, Some(&writer.cookie)).await).await;
        assert_eq!(current["build"]["current_build_id"], h4_id);
        assert_eq!(current["build"]["current_source_sequence"], version);

        let reused = test_json(
            request(
                &app,
                Method::POST,
                &build_path,
                Some(&writer.cookie),
                r#"{"trigger_type":"manual"}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(reused["reused"], true);
        assert_eq!(reused["build_id"], h4_id);
        let compile_versions: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.paper_versions WHERE workspace_id=$1 AND version_type='compile_checkpoint'",
        )
        .bind(workspace_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(compile_versions, 2);

        let saved = test_json(
            request(
                &app,
                Method::PUT,
                &file_path,
                Some(&writer.cookie),
                &serde_json::json!({"content":"\\documentclass{article}\n\\badcommand", "version":version}).to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        version = saved["version"].as_u64().unwrap();
        let failed_build = test_json(
            request(
                &app,
                Method::POST,
                &build_path,
                Some(&writer.cookie),
                r#"{"trigger_type":"auto"}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(failed_build["status"], "queued");
        let failure_worker = WorkerId::new();
        let claimed_failure = state.queue.claim(failure_worker).await.unwrap().unwrap();
        state
            .queue
            .complete_compile_failure(
                claimed_failure.id,
                failure_worker,
                false,
                serde_json::json!({"class":"compile","message":"representative failure"}),
            )
            .await
            .unwrap();
        let after_failure = test_json(get(&app, &build_path, Some(&writer.cookie)).await).await;
        assert_eq!(after_failure["build"]["current_build_id"], h4_id);
        assert_eq!(after_failure["build"]["latest_status"], "failed");
        assert_eq!(after_failure["build"]["source_sequence"], version);

        let second_checkpoint = test_json(
            request(
                &app,
                Method::POST,
                &checkpoint_path,
                Some(&writer.cookie),
                r#"{"name":"After edits"}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        let comparison = test_json(
            get(
                &app,
                &format!(
                    "{checkpoint_path}/compare?from={}&to={}",
                    first_checkpoint["id"].as_str().unwrap(),
                    second_checkpoint["id"].as_str().unwrap()
                ),
                Some(&writer.cookie),
            )
            .await,
        )
        .await;
        assert_eq!(comparison["files_changed"][0], "main.tex");
        assert!(
            comparison["text_diffs"]["main.tex"]
                .as_str()
                .unwrap()
                .contains("badcommand")
        );

        assert_eq!(
            get(&app, &checkpoint_path, Some(&outsider.cookie))
                .await
                .status(),
            StatusCode::NOT_FOUND
        );
        for denied in [&mentor, &admin] {
            assert_eq!(
                request(
                    &app,
                    Method::POST,
                    &checkpoint_path,
                    Some(&denied.cookie),
                    r#"{"name":"Denied"}"#,
                    Some("application/json"),
                )
                .await
                .status(),
                StatusCode::FORBIDDEN
            );
        }
        assert_eq!(
            get(
                &app,
                &format!("/api/v2/papers/{paper_id}/artifacts/pdf"),
                Some(&mentor.cookie),
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get(
                &app,
                &format!("/api/v2/papers/{paper_id}/artifacts/pdf"),
                Some(&admin.cookie),
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );

        pool.close().await;
        database.close().await;
    }

    async fn test_artifacts(
        state: &AppState,
        label: &str,
    ) -> Vec<persistence::PersistedArtifactV1> {
        let values = [
            (
                core_types::ArtifactKind::Pdf,
                "paper.pdf",
                "application/pdf",
                format!("pdf-{label}").into_bytes(),
            ),
            (
                core_types::ArtifactKind::Log,
                "paper.log",
                "text/plain",
                format!("log-{label}").into_bytes(),
            ),
            (
                core_types::ArtifactKind::Synctex,
                "paper.synctex.gz",
                "application/gzip",
                vec![0x1f, 0x8b, 1],
            ),
        ];
        let mut artifacts = Vec::new();
        for (kind, name, content_type, bytes) in values {
            let stored = state.blobs.put(Bytes::from(bytes)).await.unwrap();
            artifacts.push(persistence::PersistedArtifactV1 {
                artifact_id: ArtifactId::new(),
                kind,
                logical_name: LogicalPath::parse(name).unwrap(),
                blob_hash: stored.hash(),
                size_bytes: stored.size_bytes(),
                content_type: content_type.to_owned(),
            });
        }
        artifacts
    }

    type TestSocket = tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >;

    async fn ws_test_join(route: &str, cookie: &str) -> (TestSocket, Vec<u8>, String) {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut request = route.into_client_request().unwrap();
        request
            .headers_mut()
            .insert(header::COOKIE, cookie.parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        let control = match socket.next().await.unwrap().unwrap() {
            tokio_tungstenite::tungstenite::Message::Text(value) => {
                serde_json::from_str::<serde_json::Value>(&value).unwrap()
            }
            message => panic!("expected join control, got {message:?}"),
        };
        let initial = match socket.next().await.unwrap().unwrap() {
            tokio_tungstenite::tungstenite::Message::Binary(value) => {
                assert_eq!(value[0], collaboration::INITIAL_STATE);
                value[1..].to_vec()
            }
            message => panic!("expected initial state, got {message:?}"),
        };
        (
            socket,
            initial,
            control["access"].as_str().unwrap().to_owned(),
        )
    }

    fn ws_source_update(sequence: u64, update: &[u8]) -> tokio_tungstenite::tungstenite::Message {
        let mut frame = Vec::with_capacity(update.len() + 9);
        frame.push(collaboration::SOURCE_UPDATE);
        frame.extend(sequence.to_be_bytes());
        frame.extend(update);
        tokio_tungstenite::tungstenite::Message::Binary(frame.into())
    }

    async fn ws_remote_update(socket: &mut TestSocket) -> Vec<u8> {
        use futures_util::StreamExt;
        loop {
            if let tokio_tungstenite::tungstenite::Message::Binary(value) =
                socket.next().await.unwrap().unwrap()
            {
                if value[0] == collaboration::REMOTE_SOURCE_UPDATE {
                    return value[1..].to_vec();
                }
            }
        }
    }

    async fn ws_control(socket: &mut TestSocket, expected: &str) -> serde_json::Value {
        use futures_util::StreamExt;
        loop {
            if let tokio_tungstenite::tungstenite::Message::Text(value) =
                socket.next().await.unwrap().unwrap()
            {
                let value: serde_json::Value = serde_json::from_str(&value).unwrap();
                if value["type"] == expected {
                    return value;
                }
            }
        }
    }

    async fn ws_durable_ack(socket: &mut TestSocket, client_sequence: u64) {
        let value = ws_control(socket, "DURABLE_ACK").await;
        assert_eq!(value["client_seq"], client_sequence);
        assert!(value["durable_seq"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn s6_structural_history_intelligence_and_conflict_safety() {
        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, _state) = test_application().await;
        let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
        let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let other = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let writer_id = test_user_id(&pool, &writer.email).await;
        let other_id = test_user_id(&pool, &other.email).await;

        let personal = test_json(
            request(
                &app,
                Method::POST,
                "/api/v2/writer/personal-papers",
                Some(&writer.cookie),
                r#"{"name":"S6 Personal"}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        let paper_id = personal["paper"]["id"].as_str().unwrap();
        let root = format!("/api/v2/papers/{paper_id}");
        let created = test_json(
            request(
                &app,
                Method::POST,
                &format!("{root}/files"),
                Some(&writer.cookie),
                r#"{"path":"sections/methods.tex","content":"Methods","version":1}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        let file_id = created["file"]["file_id"].as_str().unwrap();
        assert_eq!(created["version"], 2);
        let undone = test_json(
            request(
                &app,
                Method::POST,
                &format!("{root}/structural-undo"),
                Some(&writer.cookie),
                "{}",
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(undone["operation_type"], "CREATE_FILE");
        assert_eq!(undone["version"], 3);
        assert!(
            test_json(get(&app, &format!("{root}/files"), Some(&writer.cookie)).await)
                .await
                .as_array()
                .unwrap()
                .iter()
                .all(|file| file["file_id"] != file_id)
        );
        let redone = test_json(
            request(
                &app,
                Method::POST,
                &format!("{root}/structural-redo"),
                Some(&writer.cookie),
                "{}",
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(redone["version"], 4);

        let renamed = test_json(
            request(
                &app,
                Method::PATCH,
                &format!("{root}/files/{file_id}/path"),
                Some(&writer.cookie),
                r#"{"path":"chapters/methods.tex","version":4}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(renamed["file"]["file_id"], file_id);
        let rename_undo = test_json(
            request(
                &app,
                Method::POST,
                &format!("{root}/structural-undo"),
                Some(&writer.cookie),
                "{}",
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(rename_undo["version"], 6);
        let restored = test_json(
            get(
                &app,
                &format!("{root}/files/{file_id}"),
                Some(&writer.cookie),
            )
            .await,
        )
        .await;
        assert_eq!(restored["file"]["path"], "sections/methods.tex");

        assert_eq!(
            test_json(
                request(
                    &app,
                    Method::DELETE,
                    &format!("{root}/files/{file_id}"),
                    Some(&writer.cookie),
                    r#"{"version":6}"#,
                    Some("application/json")
                )
                .await
            )
            .await["version"],
            7
        );
        assert_eq!(
            test_json(
                request(
                    &app,
                    Method::POST,
                    &format!("{root}/structural-undo"),
                    Some(&writer.cookie),
                    "{}",
                    Some("application/json")
                )
                .await
            )
            .await["version"],
            8
        );
        let set_main = test_json(
            request(
                &app,
                Method::POST,
                &format!("{root}/main/{file_id}"),
                Some(&writer.cookie),
                r#"{"version":8}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(set_main["version"], 9);
        assert_eq!(
            test_json(
                request(
                    &app,
                    Method::POST,
                    &format!("{root}/structural-undo"),
                    Some(&writer.cookie),
                    "{}",
                    Some("application/json")
                )
                .await
            )
            .await["version"],
            10
        );
        assert_eq!(
            test_json(get(&app, &root, Some(&writer.cookie)).await).await["main_file"],
            "main.tex"
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{root}/structural-undo"),
                Some(&other.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );

        let latex = "\\documentclass{article}\n\\usepackage{graphicx}\n\\begin{document}\n\\section{Introduction}\\label{sec:intro}\nSee \\ref{sec:intro} and \\ref{missing}. Cite \\cite{doe2026} and \\cite{missing}.\n\\bibliographystyle{plain}\\bibliography{refs}\n\\end{document}\n";
        let analysis_file = test_json(
            request(
                &app,
                Method::POST,
                &format!("{root}/files"),
                Some(&writer.cookie),
                &serde_json::json!({"path":"paper.tex","content":latex,"version":10}).to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let analysis_id = analysis_file["file"]["file_id"].as_str().unwrap();
        assert_eq!(test_json(request(&app, Method::POST, &format!("{root}/files"), Some(&writer.cookie), &serde_json::json!({"path":"refs.bib","content":"@article{doe2026,title={A Paper},author={Doe},year={2026}}","version":11}).to_string(), Some("application/json")).await).await["version"], 12);
        assert_eq!(
            test_json(
                request(
                    &app,
                    Method::POST,
                    &format!("{root}/main/{analysis_id}"),
                    Some(&writer.cookie),
                    r#"{"version":12}"#,
                    Some("application/json")
                )
                .await
            )
            .await["version"],
            13
        );
        let intelligence =
            test_json(get(&app, &format!("{root}/intelligence"), Some(&writer.cookie)).await).await;
        assert_eq!(intelligence["outline"][0]["title"], "Introduction");
        assert!(
            intelligence["labels"]
                .as_array()
                .unwrap()
                .iter()
                .any(|label| label["key"] == "sec:intro")
        );
        assert!(
            intelligence["references"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reference| reference["key"] == "missing" && reference["resolved"] == false)
        );
        assert!(
            intelligence["bibliography"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["key"] == "doe2026")
        );
        assert!(
            intelligence["diagnostics"]
                .as_array()
                .unwrap()
                .iter()
                .any(|diagnostic| diagnostic["code"] == "UnresolvedCitation")
        );
        let search = test_json(
            get(
                &app,
                &format!("{root}/search?q=Introduction&case_sensitive=false"),
                Some(&writer.cookie),
            )
            .await,
        )
        .await;
        assert_eq!(search["results"][0]["file_id"], analysis_id);

        sqlx::query("UPDATE latex_core.personal_papers SET status='frozen' WHERE id=$1")
            .bind(uuid::Uuid::parse_str(paper_id).unwrap())
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{root}/structural-undo"),
                Some(&writer.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );

        let team = test_json(request(&app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie), &serde_json::json!({"name":"S6 Team","writer_ids":[writer_id,other_id],"leader_writer_id":writer_id,"mentor_ids":[]}).to_string(), Some("application/json")).await).await;
        let team_id = team["team"]["id"].as_str().unwrap();
        let team_root = format!("/api/v2/papers/{team_id}");
        let shared = test_json(
            request(
                &app,
                Method::POST,
                &format!("{team_root}/files"),
                Some(&writer.cookie),
                r#"{"path":"shared.tex","content":"Shared","version":1}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        let shared_id = shared["file"]["file_id"].as_str().unwrap();
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &format!("{team_root}/files/{shared_id}/path"),
                Some(&other.cookie),
                r#"{"path":"renamed.tex","version":2}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{team_root}/structural-undo"),
                Some(&writer.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );

        pool.close().await;
        database.close().await;
    }

    #[tokio::test]
    async fn s7_governed_team_and_personal_restoration_is_append_only_and_epoch_safe() {
        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, state) = test_application().await;
        let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
        let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let requester = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let outsider = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let wrong_mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let writer_id = test_user_id(&pool, &writer.email).await;
        let requester_id = test_user_id(&pool, &requester.email).await;
        let mentor_id = test_user_id(&pool, &mentor.email).await;

        let created = test_json(request(
            &app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie),
            &serde_json::json!({"name":"S7 governed restore","writer_ids":[writer_id,requester_id],"leader_writer_id":writer_id,"mentor_ids":[mentor_id]}).to_string(), Some("application/json"),
        ).await).await;
        let paper_id = created["team"]["id"].as_str().unwrap();
        let workspace_id =
            uuid::Uuid::parse_str(created["team"]["workspace_id"].as_str().unwrap()).unwrap();
        let file_id = created["main_file"]["file_id"].as_str().unwrap();
        let root = format!("/api/v2/papers/{paper_id}");
        let h1 = test_json(
            request(
                &app,
                Method::POST,
                &format!("{root}/versions"),
                Some(&writer.cookie),
                r#"{"name":"H1"}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        let h1_id = h1["id"].as_str().unwrap();
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{root}/versions"),
                Some(&requester.cookie),
                r#"{"name":"unauthorized"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let h1_content = test_json(
            get(
                &app,
                &format!("{root}/files/{file_id}"),
                Some(&writer.cookie),
            )
            .await,
        )
        .await["content"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &format!("{root}/files/{file_id}"),
                Some(&writer.cookie),
                r#"{"content":"H2 durable source","version":1}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );

        let request_path = format!("{root}/restoration-requests");
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{root}/versions/{h1_id}/revert"),
                Some(&requester.cookie),
                r#"{"confirmed":true}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{root}/versions/{h1_id}/revert"),
                Some(&writer.cookie),
                r#"{"confirmed":false}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &request_path,
                Some(&outsider.cookie),
                &serde_json::json!({"target_version_id":h1_id}).to_string(),
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{root}/versions/{h1_id}/restore"),
                Some(&writer.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );

        let rejected = test_json(
            request(
                &app,
                Method::POST,
                &request_path,
                Some(&requester.cookie),
                &serde_json::json!({"target_version_id":h1_id,"reason":"Return to H1"}).to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let rejected_id = rejected["id"].as_str().unwrap();
        assert_eq!(rejected["state"], "REQUESTED");
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/restoration-requests/{rejected_id}/reject"),
                Some(&wrong_mentor.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/restoration-requests/{rejected_id}/reject"),
                Some(&mentor.cookie),
                r#"{"note":"not yet"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let leader_rejected = test_json(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/restoration-requests/{rejected_id}/reject"),
                Some(&writer.cookie),
                r#"{"note":"not yet"}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(leader_rejected["state"], "LEADER_REJECTED");
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("/api/admin/v2/restoration-requests/{rejected_id}/apply"),
                Some(&admin.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::GONE
        );

        let applied_request = test_json(
            request(
                &app,
                Method::POST,
                &request_path,
                Some(&requester.cookie),
                &serde_json::json!({"target_version_id":h1_id}).to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let applied_request_id = applied_request["id"].as_str().unwrap();
        assert_eq!(applied_request["state"], "REQUESTED");
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/restoration-requests/{applied_request_id}/apply"),
                Some(&mentor.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/restoration-requests/{applied_request_id}/apply"),
                Some(&admin.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn({
            let app = app.clone();
            async move {
                axum::serve(listener, app).await.unwrap();
            }
        });
        let route = format!("ws://{address}/api/v2/collab/{paper_id}/files/{file_id}");
        let (mut socket, _, access) = ws_test_join(&route, &writer.cookie).await;
        assert_eq!(access, "read_write");
        let before_epoch: i64 = sqlx::query_scalar(
            "SELECT document_epoch FROM latex_core.paper_collaboration_state WHERE workspace_id=$1",
        )
        .bind(workspace_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let applied_response = request(
            &app,
            Method::POST,
            &format!("/api/v2/restoration-requests/{applied_request_id}/apply"),
            Some(&writer.cookie),
            r#"{"note":"approved"}"#,
            Some("application/json"),
        )
        .await;
        assert_eq!(applied_response.status(), StatusCode::OK);
        let applied = test_json(applied_response).await;
        assert_eq!(applied["document_epoch"], before_epoch + 1);
        let epoch_signal = tokio::time::timeout(
            Duration::from_secs(2),
            ws_control(&mut socket, "PAPER_EPOCH_CHANGED"),
        )
        .await
        .expect("epoch signal timed out");
        assert_eq!(epoch_signal["document_epoch"], before_epoch + 1);
        let current = test_json(
            get(
                &app,
                &format!("{root}/files/{file_id}"),
                Some(&writer.cookie),
            )
            .await,
        )
        .await;
        assert_eq!(current["content"], h1_content);
        assert_eq!(current["version"], 3);
        let request_state: (String, Option<uuid::Uuid>) = sqlx::query_as(
            "SELECT state,applied_version_id FROM latex_core.restoration_requests WHERE id=$1",
        )
        .bind(uuid::Uuid::parse_str(applied_request_id).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(request_state.0, "APPLIED");
        assert!(request_state.1.is_some());
        let safety: (String, i64) = sqlx::query_as("SELECT version_type,workspace_version FROM latex_core.paper_versions WHERE workspace_id=$1 AND version_type='pre_restore_safety' ORDER BY version_number DESC LIMIT 1")
            .bind(workspace_id).fetch_one(&pool).await.unwrap();
        assert_eq!(safety, ("pre_restore_safety".to_owned(), 2));
        let latest: (String, i64, uuid::Uuid) = sqlx::query_as("SELECT version_type,workspace_version,created_by_user_id FROM latex_core.paper_versions WHERE workspace_id=$1 ORDER BY version_number DESC LIMIT 1")
            .bind(workspace_id).fetch_one(&pool).await.unwrap();
        assert_eq!(latest, ("team_revert".to_owned(), 3, *writer_id.as_uuid()));
        let stored = state.blobs.put(Bytes::from_static(b"stale")).await.unwrap();
        assert!(
            state
                .v2
                .persist_collaboration_batch(
                    WorkspaceId::from_uuid(workspace_id),
                    uuid::Uuid::parse_str(file_id).unwrap(),
                    u64::try_from(before_epoch).unwrap(),
                    &[persistence::CollaborationUpdateInput {
                        actor_user_id: writer_id,
                        update_bytes: vec![1]
                    }],
                    stored.hash(),
                    stored.size_bytes()
                )
                .await
                .is_err()
        );

        let personal = test_json(
            request(
                &app,
                Method::POST,
                "/api/v2/writer/personal-papers",
                Some(&writer.cookie),
                r#"{"name":"Personal restore"}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        let personal_id = personal["paper"]["id"].as_str().unwrap();
        let personal_file = personal["main_file"]["file_id"].as_str().unwrap();
        let personal_h1 = test_json(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/papers/{personal_id}/versions"),
                Some(&writer.cookie),
                r#"{"name":"P1"}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        request(
            &app,
            Method::PUT,
            &format!("/api/v2/papers/{personal_id}/files/{personal_file}"),
            Some(&writer.cookie),
            r#"{"content":"P2","version":1}"#,
            Some("application/json"),
        )
        .await;
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!(
                    "/api/v2/papers/{personal_id}/versions/{}/restore",
                    personal_h1["id"].as_str().unwrap()
                ),
                Some(&outsider.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!(
                    "/api/v2/papers/{personal_id}/versions/{}/restore",
                    personal_h1["id"].as_str().unwrap()
                ),
                Some(&writer.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );

        server.abort();
        pool.close().await;
        database.close().await;
    }

    #[tokio::test]
    async fn s7_file_policies_cover_http_websocket_hidden_and_structural_paths() {
        use futures_util::SinkExt;
        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, state) = test_application().await;
        let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
        let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let writer_id = test_user_id(&pool, &writer.email).await;
        let mentor_id = test_user_id(&pool, &mentor.email).await;
        let created = test_json(
            request(
                &app,
                Method::POST,
                "/api/admin/v2/paper-teams",
                Some(&admin.cookie),
                &serde_json::json!({"name":"Policies","writer_ids":[writer_id],"leader_writer_id":writer_id,"mentor_ids":[mentor_id]})
                    .to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let paper_id = created["team"]["id"].as_str().unwrap();
        let file_id = created["main_file"]["file_id"].as_str().unwrap();
        let root = format!("/api/v2/papers/{paper_id}");
        let file_path = format!("{root}/files/{file_id}");
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &file_path,
                Some(&writer.cookie),
                r#"{"content":"editable","version":1}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        let policy_path = format!("/api/admin/v2/paper-teams/{paper_id}/file-policies/{file_id}");
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &policy_path,
                Some(&writer.cookie),
                r#"{"policy":"CONTENT_READ_ONLY"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &policy_path,
                Some(&mentor.cookie),
                r#"{"policy":"CONTENT_READ_ONLY"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &policy_path,
                Some(&admin.cookie),
                r#"{"policy":"CONTENT_READ_ONLY"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &file_path,
                Some(&writer.cookie),
                r#"{"content":"denied","version":2}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &format!("{file_path}/path"),
                Some(&writer.cookie),
                r#"{"path":"readonly.tex","version":2}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );

        request(
            &app,
            Method::PATCH,
            &policy_path,
            Some(&admin.cookie),
            r#"{"policy":"STRUCTURE_LOCKED"}"#,
            Some("application/json"),
        )
        .await;
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &file_path,
                Some(&writer.cookie),
                r#"{"content":"structure locked edit","version":2}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(
                &app,
                Method::DELETE,
                &file_path,
                Some(&writer.cookie),
                r#"{"version":3}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &format!("{file_path}/path"),
                Some(&writer.cookie),
                r#"{"path":"locked.tex","version":3}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        request(
            &app,
            Method::PATCH,
            &policy_path,
            Some(&admin.cookie),
            r#"{"policy":"TEMPLATE_MANAGED"}"#,
            Some("application/json"),
        )
        .await;
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &file_path,
                Some(&writer.cookie),
                r#"{"content":"denied","version":3}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &file_path,
                Some(&mentor.cookie),
                r#"{"content":"denied","version":3}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &file_path,
                Some(&admin.cookie),
                r#"{"content":"denied","version":3}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );

        request(
            &app,
            Method::PATCH,
            &policy_path,
            Some(&admin.cookie),
            r#"{"policy":"CONTENT_READ_ONLY"}"#,
            Some("application/json"),
        )
        .await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn({
            let app = app.clone();
            async move {
                axum::serve(listener, app).await.unwrap();
            }
        });
        let (mut socket, _, access) = ws_test_join(
            &format!("ws://{address}/api/v2/collab/{paper_id}/files/{file_id}"),
            &writer.cookie,
        )
        .await;
        assert_eq!(access, "read_only");
        socket.send(ws_source_update(1, &[0])).await.unwrap();
        assert_eq!(ws_control(&mut socket, "ERROR").await["code"], "read_only");

        request(
            &app,
            Method::PATCH,
            &policy_path,
            Some(&admin.cookie),
            r#"{"policy":"HIDDEN_SYSTEM"}"#,
            Some("application/json"),
        )
        .await;
        let files =
            test_json(get(&app, &format!("{root}/files"), Some(&writer.cookie)).await).await;
        assert!(
            files
                .as_array()
                .unwrap()
                .iter()
                .all(|file| file["file_id"] != file_id)
        );
        assert_eq!(
            get(&app, &file_path, Some(&writer.cookie)).await.status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get(
                &app,
                &format!("/api/v2/reviews/papers/{paper_id}/files/{file_id}"),
                Some(&mentor.cookie)
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
        );

        let created_file = test_json(
            request(
                &app,
                Method::POST,
                &format!("{root}/files"),
                Some(&writer.cookie),
                r#"{"path":"undo.tex","content":"x","version":3}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        let undo_id = created_file["file"]["file_id"].as_str().unwrap();
        request(
            &app,
            Method::PATCH,
            &format!("/api/admin/v2/paper-teams/{paper_id}/file-policies/{undo_id}"),
            Some(&admin.cookie),
            r#"{"policy":"STRUCTURE_LOCKED"}"#,
            Some("application/json"),
        )
        .await;
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{root}/structural-undo"),
                Some(&writer.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            state
                .v2
                .collaboration_access(
                    writer_id,
                    uuid::Uuid::parse_str(paper_id).unwrap(),
                    uuid::Uuid::parse_str(undo_id).unwrap()
                )
                .await
                .unwrap()
                .mode,
            persistence::CollaborationAccessMode::ReadWrite
        );

        server.abort();
        pool.close().await;
        database.close().await;
    }

    #[tokio::test]
    async fn s7_template_pinning_and_team_lifecycle_are_enforced() {
        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, state) = test_application().await;
        let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
        let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let writer_id = test_user_id(&pool, &writer.email).await;
        let mentor_id = test_user_id(&pool, &mentor.email).await;
        let template_id = uuid::Uuid::new_v4();
        let template_name = format!("Governed template {template_id}");
        let main = state
            .blobs
            .put(Bytes::from_static(
                b"\\documentclass{article}\n\\begin{document}\nPinned template\n\\end{document}\n",
            ))
            .await
            .unwrap();
        let bibliography = state
            .blobs
            .put(Bytes::from_static(b"@book{core,title={LaTeX Core}}\n"))
            .await
            .unwrap();
        state
            .repo
            .create_template(
                template_id,
                &template_name,
                Some("Immutable fixture"),
                Some("main.tex"),
                &[
                    AppTemplateFileRecord {
                        path: "main.tex".to_owned(),
                        blob_hash: main.hash(),
                        size_bytes: main.size_bytes(),
                    },
                    AppTemplateFileRecord {
                        path: "references.bib".to_owned(),
                        blob_hash: bibliography.hash(),
                        size_bytes: bibliography.size_bytes(),
                    },
                ],
            )
            .await
            .unwrap();
        sqlx::query("UPDATE latex_core.templates SET policy_default='managed' WHERE id=$1")
            .bind(template_id)
            .execute(&pool)
            .await
            .unwrap();

        let created = test_json(
            request(
                &app,
                Method::POST,
                "/api/admin/v2/paper-teams",
                Some(&admin.cookie),
                &serde_json::json!({
                    "name":"Pinned Team",
                    "writer_ids":[writer_id],
                    "leader_writer_id":writer_id,
                    "mentor_ids":[mentor_id],
                    "template_id":template_id
                })
                .to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let paper_id = created["team"]["id"].as_str().unwrap();
        assert_eq!(created["files"].as_array().unwrap().len(), 2);
        assert_eq!(
            created["template_pin"]["template_id"],
            template_id.to_string()
        );
        let pin: (uuid::Uuid, String) = sqlx::query_as(
            "SELECT template_id,source_identity FROM latex_core.paper_template_pins WHERE paper_id=$1",
        )
        .bind(uuid::Uuid::parse_str(paper_id).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pin.0, template_id);
        assert_eq!(pin.1.len(), 64);
        let policies: Vec<String> = sqlx::query_scalar(
            "SELECT policy FROM latex_core.paper_file_policies WHERE workspace_id=$1 ORDER BY file_id",
        )
        .bind(uuid::Uuid::parse_str(created["team"]["workspace_id"].as_str().unwrap()).unwrap())
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(policies, vec!["TEMPLATE_MANAGED", "TEMPLATE_MANAGED"]);
        let main_file_id = created["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == "main.tex")
            .unwrap()["file_id"]
            .as_str()
            .unwrap();
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &format!("/api/v2/papers/{paper_id}/files/{main_file_id}"),
                Some(&writer.cookie),
                r#"{"content":"overwrite","version":1}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );

        let replacement_id = uuid::Uuid::new_v4();
        let replacement_main = state
            .blobs
            .put(Bytes::from_static(
                b"\\documentclass{article}\n\\begin{document}\nReplacement\n\\end{document}\n",
            ))
            .await
            .unwrap();
        let appendix = state
            .blobs
            .put(Bytes::from_static(b"Appendix\n"))
            .await
            .unwrap();
        state
            .repo
            .create_template(
                replacement_id,
                "Replacement template",
                None,
                Some("main.tex"),
                &[
                    AppTemplateFileRecord {
                        path: "main.tex".into(),
                        blob_hash: replacement_main.hash(),
                        size_bytes: replacement_main.size_bytes(),
                    },
                    AppTemplateFileRecord {
                        path: "appendix.tex".into(),
                        blob_hash: appendix.hash(),
                        size_bytes: appendix.size_bytes(),
                    },
                ],
            )
            .await
            .unwrap();
        sqlx::query("UPDATE latex_core.templates SET policy_default='managed' WHERE id=$1")
            .bind(replacement_id)
            .execute(&pool)
            .await
            .unwrap();
        let workspace_id =
            uuid::Uuid::parse_str(created["team"]["workspace_id"].as_str().unwrap()).unwrap();
        let before_head: i64 = sqlx::query_scalar(
            "SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1",
        )
        .bind(workspace_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let before_versions: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM latex_core.paper_versions WHERE workspace_id=$1",
        )
        .bind(workspace_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let change_path = format!("/api/admin/v2/paper-teams/{paper_id}/template-change");
        let preview = test_json(request(&app, Method::POST, &format!("{change_path}/preview"), Some(&admin.cookie), &serde_json::json!({"new_template_id":replacement_id,"confirm_main_file_change":true}).to_string(), Some("application/json")).await).await;
        assert_eq!(preview["can_apply"], true);
        assert!(
            preview["files_to_add"]
                .as_array()
                .unwrap()
                .iter()
                .any(|path| path == "appendix.tex")
        );
        let unchanged_pin: uuid::Uuid = sqlx::query_scalar(
            "SELECT template_id FROM latex_core.paper_template_pins WHERE paper_id=$1",
        )
        .bind(uuid::Uuid::parse_str(paper_id).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unchanged_pin, template_id);
        let unchanged_head: i64 = sqlx::query_scalar(
            "SELECT durable_version FROM latex_core.workspace_heads WHERE workspace_id=$1",
        )
        .bind(workspace_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(unchanged_head, before_head);
        let applied = test_json(request(&app, Method::POST, &format!("{change_path}/apply"), Some(&admin.cookie), &serde_json::json!({"new_template_id":replacement_id,"preview_token":preview["preview_token"],"confirm_main_file_change":true}).to_string(), Some("application/json")).await).await;
        assert_eq!(applied["resolution_method"], "MANUAL_OVERRIDE");
        let changed_pin: uuid::Uuid = sqlx::query_scalar(
            "SELECT template_id FROM latex_core.paper_template_pins WHERE paper_id=$1",
        )
        .bind(uuid::Uuid::parse_str(paper_id).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(changed_pin, replacement_id);
        let version_types: Vec<(String, Option<String>)> = sqlx::query_as("SELECT version_type,name FROM latex_core.paper_versions WHERE workspace_id=$1 ORDER BY version_number").bind(workspace_id).fetch_all(&pool).await.unwrap();
        assert_eq!(
            version_types.len(),
            usize::try_from(before_versions).unwrap() + 2
        );
        assert!(version_types.iter().any(|value| value
            == &(
                "manual_checkpoint".into(),
                Some("PRE_TEMPLATE_CHANGE".into())
            )));
        assert!(
            version_types
                .iter()
                .any(|value| value == &("template_update".into(), Some("TEMPLATE_UPDATE".into())))
        );
        let appendix_count: i64 = sqlx::query_scalar("SELECT count(*) FROM latex_core.paper_files WHERE workspace_id=$1 AND path='appendix.tex' AND NOT tombstoned").bind(workspace_id).fetch_one(&pool).await.unwrap();
        assert_eq!(appendix_count, 1);
        let appendix_file_id: uuid::Uuid = sqlx::query_scalar("SELECT file_id FROM latex_core.paper_files WHERE workspace_id=$1 AND path='appendix.tex' AND NOT tombstoned").bind(workspace_id).fetch_one(&pool).await.unwrap();
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &format!("/api/admin/v2/paper-teams/{paper_id}/file-policies/{appendix_file_id}"),
                Some(&admin.cookie),
                r#"{"policy":"EDITABLE"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(
                &app,
                Method::PUT,
                &format!("/api/v2/papers/{paper_id}/files/{appendix_file_id}"),
                Some(&writer.cookie),
                r#"{"content":"Writer appendix","version":2}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        let conflicting_id = uuid::Uuid::new_v4();
        let conflicting_appendix = state
            .blobs
            .put(Bytes::from_static(b"Template appendix replacement\n"))
            .await
            .unwrap();
        state
            .repo
            .create_template(
                conflicting_id,
                "Conflicting template",
                None,
                Some("main.tex"),
                &[
                    AppTemplateFileRecord {
                        path: "main.tex".into(),
                        blob_hash: replacement_main.hash(),
                        size_bytes: replacement_main.size_bytes(),
                    },
                    AppTemplateFileRecord {
                        path: "appendix.tex".into(),
                        blob_hash: conflicting_appendix.hash(),
                        size_bytes: conflicting_appendix.size_bytes(),
                    },
                ],
            )
            .await
            .unwrap();
        let conflict_preview = test_json(request(&app, Method::POST, &format!("{change_path}/preview"), Some(&admin.cookie), &serde_json::json!({"new_template_id":conflicting_id,"confirm_main_file_change":true}).to_string(), Some("application/json")).await).await;
        assert_eq!(conflict_preview["can_apply"], false);
        assert!(
            conflict_preview["writer_modified_conflicts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value["path"] == "appendix.tex")
        );
        let retained_pin: uuid::Uuid = sqlx::query_scalar(
            "SELECT template_id FROM latex_core.paper_template_pins WHERE paper_id=$1",
        )
        .bind(uuid::Uuid::parse_str(paper_id).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(retained_pin, replacement_id);

        let status_path = format!("/api/admin/v2/paper-teams/{paper_id}/status");
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &status_path,
                Some(&admin.cookie),
                r#"{"status":"frozen"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/papers/{paper_id}/files"),
                Some(&writer.cookie),
                r#"{"path":"frozen.tex","content":"x","version":1}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &status_path,
                Some(&admin.cookie),
                r#"{"status":"active"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(
                &app,
                Method::PATCH,
                &status_path,
                Some(&admin.cookie),
                r#"{"status":"archived"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::OK
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/papers/{paper_id}/files"),
                Some(&writer.cookie),
                r#"{"path":"archived.tex","content":"x","version":1}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );

        pool.close().await;
        database.close().await;
    }

    #[tokio::test]
    async fn s7_bounded_concurrency_smoke_12_websocket_clients() {
        use futures_util::{SinkExt, StreamExt};
        use yrs::{Doc, GetString, ReadTxn, Text, Transact, Update, updates::decoder::Decode};

        const CLIENTS: usize = 12;
        let _guard = SERVER_TEST_LOCK.lock().await;
        let (database, pool, app, _storage, state) = test_application().await;
        let admin = fixture(&app, &database, "admin", Some(GlobalRole::Admin)).await;
        let writer = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let writer_id = test_user_id(&pool, &writer.email).await;
        let created = test_json(
            request(
                &app,
                Method::POST,
                "/api/admin/v2/paper-teams",
                Some(&admin.cookie),
                &serde_json::json!({"name":"12-client smoke","writer_ids":[writer_id],"leader_writer_id":writer_id,"mentor_ids":[]}).to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let paper_id = created["team"]["id"].as_str().unwrap();
        let workspace_id = WorkspaceId::from_uuid(
            uuid::Uuid::parse_str(created["team"]["workspace_id"].as_str().unwrap()).unwrap(),
        );
        let file_id = created["main_file"]["file_id"].as_str().unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn({
            let app = app.clone();
            async move {
                axum::serve(listener, app).await.unwrap();
            }
        });
        let route = format!("ws://{address}/api/v2/collab/{paper_id}/files/{file_id}");
        let started = std::time::Instant::now();
        let mut clients = Vec::new();
        for _ in 0..CLIENTS {
            let (socket, initial, access) = ws_test_join(&route, &writer.cookie).await;
            assert_eq!(access, "read_write");
            let doc = Doc::new();
            let text = doc.get_or_insert_text("source");
            doc.transact_mut()
                .apply_update(Update::decode_v1(&initial).unwrap())
                .unwrap();
            clients.push((socket, doc, text));
        }
        for (index, (socket, doc, text)) in clients.iter_mut().enumerate() {
            let before = doc.transact().state_vector();
            let end = text.len(&doc.transact());
            text.insert(&mut doc.transact_mut(), end, &format!(" [{index:02}]"));
            let update = doc.transact().encode_diff_v1(&before);
            socket
                .send(ws_source_update(u64::try_from(index + 1).unwrap(), &update))
                .await
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(8), async {
            for (index, (socket, doc, _)) in clients.iter_mut().enumerate() {
                let mut remotes = 0;
                let mut acknowledged = false;
                while remotes < CLIENTS - 1 || !acknowledged {
                    match socket.next().await.unwrap().unwrap() {
                        tokio_tungstenite::tungstenite::Message::Binary(value)
                            if value[0] == collaboration::REMOTE_SOURCE_UPDATE =>
                        {
                            doc.transact_mut()
                                .apply_update(Update::decode_v1(&value[1..]).unwrap())
                                .unwrap();
                            remotes += 1;
                        }
                        tokio_tungstenite::tungstenite::Message::Text(value) => {
                            let value: serde_json::Value = serde_json::from_str(&value).unwrap();
                            if value["type"] == "DURABLE_ACK"
                                && value["client_seq"] == u64::try_from(index + 1).unwrap()
                            {
                                acknowledged = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
        })
        .await
        .expect("12-client convergence timed out");
        let exact = clients[0].2.get_string(&clients[0].1.transact());
        assert!(
            clients
                .iter()
                .all(|(_, doc, text)| text.get_string(&doc.transact()) == exact)
        );
        let canonical = state
            .workspaces
            .read_file(workspace_id, &LogicalPath::parse("main.tex").unwrap())
            .await
            .unwrap();
        assert_eq!(String::from_utf8(canonical.to_vec()).unwrap(), exact);
        assert!(started.elapsed() < Duration::from_secs(8));
        for (socket, _, _) in &mut clients {
            let _ = socket.close(None).await;
        }
        server.abort();
        pool.close().await;
        database.close().await;
    }

    async fn test_application() -> (Database, PgPool, Router, TempDir, AppState) {
        let url = env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL is required");
        let database =
            Database::connect(DatabaseConfig::new(&url, 1, 5, Duration::from_secs(5)).unwrap())
                .await
                .unwrap();
        database.migrate().await.unwrap();
        let pool = PgPool::connect(&url).await.unwrap();
        let storage = tempfile::tempdir().unwrap();
        let blobs = Arc::new(
            FsBlobStore::open(storage.path(), FsBlobStoreConfig::development_default())
                .await
                .unwrap(),
        );
        let v2 = V2Repository::new(database.clone());
        let workspaces = WorkspaceService::new(
            persistence::PostgresWorkspaceRepository::new(database.clone()),
            blobs.clone(),
        );
        let collaboration =
            collaboration::CollaborationHub::new(v2.clone(), workspaces.clone(), blobs.clone());
        let state = AppState {
            repo: AppRepository::new(database.clone()),
            v2,
            institution: InstitutionRepository::new(database.clone()),
            workspaces,
            queue: PostgresCompileQueue::new(
                database.clone(),
                QueueLimits::new(2, 1, 8, Duration::from_secs(120), 3).unwrap(),
            ),
            blobs,
            collaboration,
            environment: TexEnvironmentId::parse("development-env").unwrap(),
            cookie_secure: false,
            allow_registration: false,
            session_seconds: 3600,
        };
        (database, pool, router(state.clone()), storage, state)
    }

    async fn fixture(
        app: &Router,
        database: &Database,
        account_type: &str,
        role: Option<GlobalRole>,
    ) -> Fixture {
        let repo = AppRepository::new(database.clone());
        let email = format!("{}@c4.example", uuid::Uuid::new_v4());
        let password_hash = auth::hash_password(PASSWORD).unwrap();
        let user = repo.create_account(&email, &password_hash).await.unwrap();
        repo.set_user_account_type(&email, account_type)
            .await
            .unwrap();
        if let Some(role) = role {
            V2Repository::new(database.clone())
                .set_global_role(user.user_id, role)
                .await
                .unwrap();
        }
        let response = login_request(app, &email, PASSWORD).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        Fixture {
            email,
            cookie: response_cookie(&response),
        }
    }

    async fn assert_login_redirect(app: &Router, email: &str, expected: &str) {
        let response = login_request(app, email, PASSWORD).await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(response.headers()[header::LOCATION], expected);
    }

    async fn login_request(app: &Router, email: &str, password: &str) -> Response {
        request(
            app,
            Method::POST,
            "/login",
            None,
            &format!("email={email}&password={password}"),
            Some("application/x-www-form-urlencoded"),
        )
        .await
    }

    fn response_cookie(response: &Response) -> String {
        response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .find_map(|value| {
                let value = value.to_str().ok()?;
                value
                    .starts_with(COOKIE)
                    .then(|| value.split(';').next().unwrap().to_owned())
            })
            .expect("canonical session cookie")
    }

    async fn assert_routes(app: &Router, cookie: &str, routes: &[(&str, u16)]) {
        for (path, expected) in routes {
            assert_eq!(
                get(app, path, Some(cookie)).await.status().as_u16(),
                *expected,
                "unexpected status for {path}"
            );
        }
    }

    async fn get(app: &Router, path: &str, cookie: Option<&str>) -> Response {
        request(app, Method::GET, path, cookie, "", None).await
    }

    fn institution_batch_multipart(files: &[(&str, Vec<u8>)]) -> (Vec<u8>, String) {
        const BOUNDARY: &str = "latex-core-institution-batch-test-boundary";
        let mut body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"operation\"\r\n\r\nADD\r\n"
        )
        .into_bytes();
        for (filename, bytes) in files {
            body.extend_from_slice(
                format!(
                    "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"files[]\"; filename=\"{filename}\"\r\nContent-Type: text/csv\r\n\r\n"
                )
                .as_bytes(),
            );
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
        (body, format!("multipart/form-data; boundary={BOUNDARY}"))
    }

    async fn request(
        app: &Router,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        body: &str,
        content_type: Option<&str>,
    ) -> Response {
        request_bytes(
            app,
            method,
            path,
            cookie,
            body.as_bytes().to_vec(),
            content_type,
        )
        .await
    }

    async fn request_bytes(
        app: &Router,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        body: Vec<u8>,
        content_type: Option<&str>,
    ) -> Response {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(cookie) = cookie {
            builder = builder.header(header::COOKIE, cookie);
        }
        if let Some(content_type) = content_type {
            builder = builder.header(header::CONTENT_TYPE, content_type);
        }
        app.clone()
            .oneshot(builder.body(Body::from(body)).unwrap())
            .await
            .unwrap()
    }
}
