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
    extract::{Form, Path, Query, State, WebSocketUpgrade},
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
    AccountType, AppError, AppRepository, AppSessionRecord, ChangeSetPublishResult, Database,
    DatabaseConfig, EnqueueCompileJobV1, ExactRestoreState, FilePolicy, GlobalRole, GroupType,
    PostgresCompileQueue, ProjectAccess, ProjectRoles, PublishResult, QueueLimits, TeamFileRecord,
    TemplateSeedFile, V2BuildRequest, V2Error, V2FilePolicy, V2Repository,
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
    password: String,
    role: String,
}
#[derive(Deserialize)]
struct V2RoleInput {
    role: String,
}
#[derive(Deserialize)]
struct V2PaperTeamInput {
    name: String,
    template_id: Option<uuid::Uuid>,
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
            "/api/admin/v2/paper-teams",
            get(admin_v2_paper_teams).post(admin_v2_create_paper_team),
        )
        .route("/api/admin/v2/paper-teams/{id}", get(admin_v2_paper_team))
        .route(
            "/api/admin/v2/paper-teams/{id}/members",
            post(admin_v2_add_paper_team_member),
        )
        .route(
            "/api/admin/v2/paper-teams/{id}/members/{user_id}",
            axum::routing::delete(admin_v2_remove_paper_team_member),
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
            post(admin_v2_reject_restoration),
        )
        .route(
            "/api/admin/v2/restoration-requests/{request_id}/apply",
            post(admin_v2_apply_restoration),
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
            "/api/v2/papers/{paper_id}/restoration-requests",
            get(v2_restoration_requests).post(v2_create_restoration_request),
        )
        .route(
            "/api/v2/restoration-requests/{request_id}/submit",
            post(v2_submit_restoration_request),
        )
        .route(
            "/api/v2/restoration-requests/{request_id}/endorse",
            post(v2_mentor_endorse_restoration),
        )
        .route(
            "/api/v2/restoration-requests/{request_id}/reject",
            post(v2_mentor_reject_restoration),
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
        .layer(RequestBodyLimitLayer::new(archive::MAX_ARCHIVE_BYTES))
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
        Ok(user) => session_response(&state, &headers, user.user_id, user.email).await,
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
    let Some((user, email)) = (match valid_credentials(&state, &input).await {
        Ok(value) => value,
        Err(response) => return response,
    }) else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    session_response(&state, &headers, user, email).await
}

async fn browser_login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(input): Form<Credentials>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let Some((user, _email)) = (match valid_credentials(&state, &input).await {
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
        Ok(kind) => landing_path(kind),
        Err(response) => return response,
    };
    redirect_with_cookies(location, cookies)
}

async fn valid_credentials(
    state: &AppState,
    input: &Credentials,
) -> Result<Option<(UserId, String)>, Response> {
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
    Ok(valid.then_some((user.user_id, user.email)))
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

const INITIAL_TEX: &str = "\\documentclass{article}\n\\begin{document}\n\n\\end{document}\n";

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
    let password_hash = match auth::hash_password(&input.password) {
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
        .create_v2_account(&email, &password_hash, role)
        .await
    {
        Ok(user) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "user_id": user.user_id,
                "email": user.email,
                "enabled": user.enabled,
                "role": role
            })),
        )
            .into_response(),
        Err(AppError::Conflict) => error(StatusCode::CONFLICT, "email already exists"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "account creation failed"),
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
    ) {
        (Ok(team), Ok(members), Ok(template_pin)) => {
            Json(serde_json::json!({"team":team,"members":members,"template_pin":template_pin}))
                .into_response()
        }
        (Err(error_value), _, _) | (_, Err(error_value), _) | (_, _, Err(error_value)) => {
            v2_error(error_value)
        }
    }
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
    if let Some(template_id) = input.template_id {
        let template = match state.repo.template(template_id).await {
            Ok(value) => value,
            Err(_) => return error(StatusCode::NOT_FOUND, "template not found"),
        };
        let records = match state.repo.template_files(template_id).await {
            Ok(value) if !value.is_empty() => value,
            Ok(_) => return error(StatusCode::CONFLICT, "template has no files"),
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "template lookup failed"),
        };
        let main_path = match template
            .main_file
            .as_deref()
            .map(LogicalPath::parse)
            .transpose()
        {
            Ok(Some(value)) => value,
            Ok(None) => return error(StatusCode::CONFLICT, "template has no main file"),
            Err(_) => return error(StatusCode::CONFLICT, "template main file is invalid"),
        };
        let policy = match template.policy_default.as_str() {
            "editable" => V2FilePolicy::Editable,
            "read_only" => V2FilePolicy::ContentReadOnly,
            "managed" => V2FilePolicy::TemplateManaged,
            _ => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "template policy is invalid",
                );
            }
        };
        let mut identity_material = format!(
            "{template_id}:{}:{}",
            template.main_file.as_deref().unwrap_or(""),
            template.policy_default
        );
        let mut seeds = Vec::with_capacity(records.len());
        for record in records {
            let path = match LogicalPath::parse(&record.path) {
                Ok(value) => value,
                Err(_) => return error(StatusCode::CONFLICT, "template contains an invalid path"),
            };
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
        let source_identity = digest(&identity_material);
        return match state.v2.create_template_paper_team(
            principal.user_id(), principal.session.tenant_id, WorkspaceId::new(), &input.name,
            &writer_ids, &mentor_ids, template_id, &source_identity, &main_path, &seeds,
        ).await {
            Ok((team, files)) => (StatusCode::CREATED, Json(serde_json::json!({"team":team,"files":files,"template_pin":{"template_id":template_id,"source_identity":source_identity}}))).into_response(),
            Err(value) => v2_error(value),
        };
    }
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
        .create_initialized_paper_team(
            principal.user_id(),
            principal.session.tenant_id,
            WorkspaceId::new(),
            &input.name,
            &writer_ids,
            &mentor_ids,
            &main_path,
            stored.hash(),
            stored.size_bytes(),
        )
        .await
    {
        Ok((team, file)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({"team":team,"main_file":file})),
        )
            .into_response(),
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

async fn admin_v2_reject_restoration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<uuid::Uuid>,
    Json(input): Json<GovernanceDecisionInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .admin_reject_restoration(principal.user_id(), request_id, input.note.as_deref())
        .await
    {
        Ok(request) => Json(request).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn admin_v2_apply_restoration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<uuid::Uuid>,
    Json(input): Json<GovernanceDecisionInput>,
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
            "V2 Admin required for governed restoration",
        );
    }
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
            None => return error(StatusCode::NOT_FOUND, "restoration request not found"),
        },
        Err(value) => return v2_error(value),
    };
    if request.state != "AWAITING_ADMIN_REVIEW" {
        return error(
            StatusCode::CONFLICT,
            "restoration request is not awaiting Admin review",
        );
    }
    let team = match state.v2.paper_team(request.paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let paper = persistence::WriterPaper {
        id: team.id,
        workspace_id: team.workspace_id,
        name: team.name,
        kind: persistence::PaperKind::Team,
        status: team.status,
        updated_at: team.updated_at,
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

async fn v2_submit_restoration_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<uuid::Uuid>,
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
        .submit_restoration_request(principal.user_id(), request_id)
        .await
    {
        Ok(request) => Json(request).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn v2_assigned_restoration_requests(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let principal = match principal_auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !matches!(principal.kind, PrincipalKind::V2(GlobalRole::Mentor)) {
        return error(StatusCode::FORBIDDEN, "V2 Mentor required");
    }
    match state
        .v2
        .restoration_requests_for_actor(principal.user_id(), None)
        .await
    {
        Ok(requests) => Json(requests).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn v2_mentor_endorse_restoration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<uuid::Uuid>,
    Json(input): Json<GovernanceDecisionInput>,
) -> Response {
    v2_mentor_restoration_decision(state, headers, request_id, input, true).await
}

async fn v2_mentor_reject_restoration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(request_id): Path<uuid::Uuid>,
    Json(input): Json<GovernanceDecisionInput>,
) -> Response {
    v2_mentor_restoration_decision(state, headers, request_id, input, false).await
}

async fn v2_mentor_restoration_decision(
    state: AppState,
    headers: HeaderMap,
    request_id: uuid::Uuid,
    input: GovernanceDecisionInput,
    endorse: bool,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let principal = match principal_auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !matches!(principal.kind, PrincipalKind::V2(GlobalRole::Mentor)) {
        return error(StatusCode::FORBIDDEN, "V2 Mentor required");
    }
    match state
        .v2
        .mentor_decide_restoration(
            principal.user_id(),
            request_id,
            endorse,
            input.note.as_deref(),
        )
        .await
    {
        Ok(request) => Json(request).into_response(),
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
            "Team Paper restoration requires Mentor and Admin governance",
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
    let password = input.password.unwrap_or_else(auth::temporary_password);
    let hash = match auth::hash_password(&password) {
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
    let password = input.password.unwrap_or_else(auth::temporary_password);
    let hash = match auth::hash_password(&password) {
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
    match state.repo.list_templates().await { Ok(templates) => Json(templates.into_iter().map(|template| serde_json::json!({"id":template.id.to_string(),"name":template.name,"description":template.description,"main_file":template.main_file,"policy_default":template.policy_default,"created_at":template.created_at,"update_status":"Template update unavailable in this RC"})).collect::<Vec<_>>()).into_response(), Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "persistence failure") }
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
) -> Response {
    let cookies = match create_session_headers(state, headers, user).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    (
        StatusCode::CREATED,
        cookies,
        Json(match identity_for_user(state, user, email).await {
            Ok(identity) => identity,
            Err(response) => return response,
        }),
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
    identity_for_kind(state, user, email, kind).await
}

async fn identity_for_kind(
    state: &AppState,
    user: UserId,
    email: String,
    kind: PrincipalKind,
) -> Result<UserWire, Response> {
    let (account_type, persona, landing_path, v2_role, is_admin, has_mentor_projects) = match kind {
        PrincipalKind::V2(role) => (
            role.as_str(),
            role.as_str(),
            landing_path(kind),
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
                landing_path(kind),
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

fn redirect_with_cookies(location: &'static str, cookies: HeaderMap) -> Response {
    let mut response = (StatusCode::SEE_OTHER, [(header::LOCATION, location)]).into_response();
    response.headers_mut().extend(cookies);
    response
}

async fn ui(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match principal_auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
            return Html(login_html(None)).into_response();
        }
        Err(response) => return response,
    };
    redirect_with_cookies(landing_path(principal.kind), HeaderMap::new())
}

async fn admin_ui(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match principal_auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
            return Html(login_html(None)).into_response();
        }
        Err(response) => return response,
    };
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
    let principal = match principal_auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
            return Html(login_html(None)).into_response();
        }
        Err(response) => return response,
    };
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
    let principal = match principal_auth(state, headers).await {
        Ok(value) => value,
        Err(response) if response.status() == StatusCode::UNAUTHORIZED => {
            return Html(login_html(None)).into_response();
        }
        Err(response) => return response,
    };
    match principal.kind {
        PrincipalKind::V2(role) if role == required => Html(html).into_response(),
        PrincipalKind::V2(_) | PrincipalKind::Legacy(_) => {
            error(StatusCode::FORBIDDEN, "role-specific access required")
        }
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
    fn v2_shells_expose_the_frozen_information_architecture_only() {
        let writer = writer_html();
        for required in [
            "MY PAPERS",
            "TEAM PAPERS",
            "FILES",
            "EDITOR",
            "PDF",
            "PROBLEMS",
            "REVIEWS",
            "HISTORY",
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
            "ASSIGNED REVIEWS",
            "ACTIVITY",
            "REVIEW ROUNDS",
            "READ-ONLY SOURCE",
            "PDF",
            "REVIEW THREADS",
            "APPROVALS",
            "VERSIONS",
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
            "<textarea",
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
            "PAPER TEAMS",
            "TEMPLATES",
            "FILE POLICIES",
            "VERSIONS",
            "RESTORATION REQUESTS",
            "REVIEWS",
            "BUILD QUEUE",
            "AUDIT",
            "SYSTEM",
        ] {
            assert!(admin.contains(required), "missing Admin section {required}");
        }
        assert!(!admin.contains("LEGACY RESEARCH GROUPS"));
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
        let mentor_id = test_user_id(&pool, &mentor.email).await;
        let admin_id = test_user_id(&pool, &admin.email).await;
        let team_response = request(
            &app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie),
            &serde_json::json!({"name":"S2 Team","writer_ids":[writer_id],"mentor_ids":[mentor_id]}).to_string(),
            Some("application/json"),
        ).await;
        assert_eq!(team_response.status(), StatusCode::CREATED);
        let team = test_json(team_response).await;
        let team_id = team["team"]["id"].as_str().unwrap().to_owned();
        assert_eq!(team["main_file"]["path"], "main.tex");
        assert_eq!(
            request(
                &app,
                Method::POST,
                "/api/admin/v2/paper-teams",
                Some(&admin.cookie),
                &serde_json::json!({"name":"Wrong role","writer_ids":[admin_id],"mentor_ids":[]})
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
                    r#"{"name":"Denied","writer_ids":[],"mentor_ids":[]}"#,
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
        let mentor_id = test_user_id(&pool, &mentor.email).await;

        let created = test_json(
            request(
                &app,
                Method::POST,
                "/api/admin/v2/paper-teams",
                Some(&admin.cookie),
                &serde_json::json!({"name":"S5 Review Team","writer_ids":[writer_id],"mentor_ids":[mentor_id]}).to_string(),
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

        let round = request(
            &app,
            Method::POST,
            &format!("{review_root}/rounds"),
            Some(&mentor.cookie),
            "{}",
            Some("application/json"),
        )
        .await;
        assert_eq!(round.status(), StatusCode::CREATED);
        let round = test_json(round).await;
        let round_id = round["id"].as_str().unwrap();
        let source_anchor = serde_json::json!({
            "file_id":file_id,"encoded_relative_start":[1],"encoded_relative_end":[2],
            "quoted_text":"article","context_hash":"0".repeat(64),"source_sequence":1,
            "source_version_id":null,"document_epoch":1
        });
        let comment = serde_json::json!({"thread_type":"COMMENT","message":"Clarify this paragraph","severity":"MINOR","category":"WRITING","assigned_writer_user_id":null,"due_at":null,"source_anchor":source_anchor,"pdf_anchor":null,"suggested_replacement":null,"section_label":null});
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
                r#"{"state":"ADDRESSED"}"#,
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
                &format!("{review_root}/threads/{thread_id}/state"),
                Some(&writer.cookie),
                r#"{"state":"RESOLVED"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/threads/{thread_id}/state"),
                Some(&mentor.cookie),
                r#"{"state":"RESOLVED"}"#,
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
                &format!("{review_root}/threads/{thread_id}/state"),
                Some(&mentor.cookie),
                r#"{"state":"REOPENED"}"#,
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
                &format!("{review_root}/threads/{thread_id}/state"),
                Some(&mentor.cookie),
                r#"{"state":"RESOLVED"}"#,
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::CONFLICT
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
        let suggestion = serde_json::json!({"thread_type":"SUGGESTED_REPLACEMENT","message":"Use a stronger phrase","severity":"MAJOR","category":"WRITING","assigned_writer_user_id":writer_id,"due_at":null,"source_anchor":source_anchor,"pdf_anchor":null,"suggested_replacement":"replacement","section_label":null});
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

        let paper_approval = serde_json::json!({"thread_type":"PAPER_APPROVAL","message":"Approved for this exact version","severity":"NOTE","category":"SUBMISSION_REQUIREMENT","assigned_writer_user_id":null,"due_at":null,"source_anchor":null,"pdf_anchor":null,"suggested_replacement":null,"section_label":null});
        let paper_approval_id = test_json(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/threads"),
                Some(&mentor.cookie),
                &paper_approval.to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await["thread_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let approval_identity: (Option<uuid::Uuid>, Option<uuid::Uuid>, Option<String>) =
            sqlx::query_as("SELECT approved_version_id,approved_build_id,approved_state_hash FROM latex_core.review_threads WHERE id=$1")
                .bind(uuid::Uuid::parse_str(&paper_approval_id).unwrap())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(approval_identity.0.is_some() && approval_identity.1.is_some());
        let approved_state_hash = approval_identity.2.unwrap();

        let blocking = serde_json::json!({"thread_type":"CHANGE_REQUEST","message":"Blocking request","severity":"BLOCKING","category":"METHODOLOGY","assigned_writer_user_id":writer_id,"due_at":null,"source_anchor":source_anchor,"pdf_anchor":null,"suggested_replacement":null,"section_label":null});
        let blocking_id = test_json(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/threads"),
                Some(&mentor.cookie),
                &blocking.to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await["thread_id"]
            .as_str()
            .unwrap()
            .to_owned();
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
            StatusCode::CONFLICT
        );
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("{review_root}/threads/{blocking_id}/state"),
                Some(&writer.cookie),
                r#"{"state":"ADDRESSED"}"#,
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
                &format!("{review_root}/threads/{blocking_id}/state"),
                Some(&mentor.cookie),
                r#"{"state":"RESOLVED"}"#,
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
            StatusCode::OK
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
        assert_ne!(approved_state_hash, newer.snapshot_id().to_hex());

        assert_eq!(
            get(
                &app,
                &format!("{review_root}/threads"),
                Some(&other_writer.cookie)
            )
            .await
            .status(),
            StatusCode::NOT_FOUND
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
            let bytes = zstd::stream::decode_all(std::io::Cursor::new(compressed)).unwrap();
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

        let team = test_json(request(&app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie), &serde_json::json!({"name":"S6 Team","writer_ids":[writer_id,other_id],"mentor_ids":[]}).to_string(), Some("application/json")).await).await;
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
        let outsider = fixture(&app, &database, "student", Some(GlobalRole::Writer)).await;
        let mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let wrong_mentor = fixture(&app, &database, "professor", Some(GlobalRole::Mentor)).await;
        let writer_id = test_user_id(&pool, &writer.email).await;
        let mentor_id = test_user_id(&pool, &mentor.email).await;

        let created = test_json(request(
            &app, Method::POST, "/api/admin/v2/paper-teams", Some(&admin.cookie),
            &serde_json::json!({"name":"S7 governed restore","writer_ids":[writer_id],"mentor_ids":[mentor_id]}).to_string(), Some("application/json"),
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
                Some(&writer.cookie),
                &serde_json::json!({"target_version_id":h1_id,"reason":"Return to H1"}).to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let rejected_id = rejected["id"].as_str().unwrap();
        assert_eq!(rejected["state"], "DRAFT");
        let submitted = test_json(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/restoration-requests/{rejected_id}/submit"),
                Some(&writer.cookie),
                "{}",
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(submitted["state"], "AWAITING_MENTOR_REVIEW");
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
        let mentor_rejected = test_json(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/restoration-requests/{rejected_id}/reject"),
                Some(&mentor.cookie),
                r#"{"note":"not yet"}"#,
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(mentor_rejected["state"], "MENTOR_REJECTED");

        let admin_reject = test_json(
            request(
                &app,
                Method::POST,
                &request_path,
                Some(&writer.cookie),
                &serde_json::json!({"target_version_id":h1_id}).to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let admin_reject_id = admin_reject["id"].as_str().unwrap();
        request(
            &app,
            Method::POST,
            &format!("/api/v2/restoration-requests/{admin_reject_id}/submit"),
            Some(&writer.cookie),
            "{}",
            Some("application/json"),
        )
        .await;
        let endorsed = test_json(
            request(
                &app,
                Method::POST,
                &format!("/api/v2/restoration-requests/{admin_reject_id}/endorse"),
                Some(&mentor.cookie),
                "{}",
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(endorsed["state"], "AWAITING_ADMIN_REVIEW");
        assert_eq!(
            request(
                &app,
                Method::POST,
                &format!("/api/admin/v2/restoration-requests/{admin_reject_id}/apply"),
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
                &format!("/api/admin/v2/restoration-requests/{admin_reject_id}/apply"),
                Some(&writer.cookie),
                "{}",
                Some("application/json")
            )
            .await
            .status(),
            StatusCode::FORBIDDEN
        );
        let admin_rejected = test_json(
            request(
                &app,
                Method::POST,
                &format!("/api/admin/v2/restoration-requests/{admin_reject_id}/reject"),
                Some(&admin.cookie),
                "{}",
                Some("application/json"),
            )
            .await,
        )
        .await;
        assert_eq!(admin_rejected["state"], "ADMIN_REJECTED");

        let applied_request = test_json(
            request(
                &app,
                Method::POST,
                &request_path,
                Some(&writer.cookie),
                &serde_json::json!({"target_version_id":h1_id}).to_string(),
                Some("application/json"),
            )
            .await,
        )
        .await;
        let applied_request_id = applied_request["id"].as_str().unwrap();
        request(
            &app,
            Method::POST,
            &format!("/api/v2/restoration-requests/{applied_request_id}/submit"),
            Some(&writer.cookie),
            "{}",
            Some("application/json"),
        )
        .await;
        request(
            &app,
            Method::POST,
            &format!("/api/v2/restoration-requests/{applied_request_id}/endorse"),
            Some(&mentor.cookie),
            "{}",
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
            &format!("/api/admin/v2/restoration-requests/{applied_request_id}/apply"),
            Some(&admin.cookie),
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
        let latest: (String, i64) = sqlx::query_as("SELECT version_type,workspace_version FROM latex_core.paper_versions WHERE workspace_id=$1 ORDER BY version_number DESC LIMIT 1")
            .bind(workspace_id).fetch_one(&pool).await.unwrap();
        assert_eq!(latest, ("admin_restoration".to_owned(), 3));
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
                &serde_json::json!({"name":"Policies","writer_ids":[writer_id],"mentor_ids":[mentor_id]})
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
                    persistence::AppTemplateFileRecord {
                        path: "main.tex".to_owned(),
                        blob_hash: main.hash(),
                        size_bytes: main.size_bytes(),
                    },
                    persistence::AppTemplateFileRecord {
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
                &serde_json::json!({"name":"12-client smoke","writer_ids":[writer_id],"mentor_ids":[]}).to_string(),
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

    async fn request(
        app: &Router,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        body: &str,
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
            .oneshot(builder.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap()
    }
}
