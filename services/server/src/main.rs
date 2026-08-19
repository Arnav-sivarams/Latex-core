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
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use blob_store::{BlobStore, FsBlobStore, FsBlobStoreConfig};
use core_types::{
    ArtifactId, CompileKeyMaterialV1, CostClass, IdempotencyKey, JobId, LatexmkProfileId,
    LogicalPath, ShellPolicy, TexEngine, TexEnvironmentId, UserId, WorkspaceId, WorkspaceVersion,
};
use persistence::{
    AppError, AppRepository, ChangeSetPublishResult, Database, DatabaseConfig, EnqueueCompileJobV1,
    FilePolicy, GroupType, PostgresCompileQueue, ProjectAccess, ProjectRoles, PublishResult,
    QueueLimits, TeamFileRecord,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, env, net::SocketAddr, str::FromStr, sync::Arc, time::Duration};
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
    workspaces: WorkspaceService,
    queue: PostgresCompileQueue,
    blobs: Arc<FsBlobStore>,
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
    capabilities: IdentityCapabilitiesWire,
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
    error: &'static str,
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
    let state = AppState {
        repo: AppRepository::new(database.clone()),
        workspaces: WorkspaceService::new(
            persistence::PostgresWorkspaceRepository::new(database),
            blobs.clone(),
        ),
        queue,
        blobs,
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

fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(ui))
        .route("/admin", get(admin_ui))
        .route("/static/styles.css", get(styles))
        .route("/static/app.js", get(app_js))
        .route("/static/api.js", get(api_js))
        .route("/static/state.js", get(state_js))
        .route("/api/auth/register", post(register))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
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
    let email = match auth::normalized_email(&input.email) {
        Ok(v) => v,
        Err(_) => return error(StatusCode::UNAUTHORIZED, "invalid credentials"),
    };
    let Ok(Some(user)) = state.repo.user_by_email(&email).await else {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    if !user.enabled {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    }
    let Ok(valid) = auth::verify_password(&input.password, &user.password_hash) else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, "authentication failure");
    };
    if !valid {
        return error(StatusCode::UNAUTHORIZED, "invalid credentials");
    };
    session_response(&state, &headers, user.user_id, user.email).await
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    };
    for token in named_cookies(&headers, COOKIE) {
        if state.repo.delete_session(&digest(&token)).await.is_err() {
            return error(StatusCode::INTERNAL_SERVER_ERROR, "session failure");
        }
    }
    let cookies = match expired_session_cookie_headers(state.cookie_secure) {
        Ok(value) => value,
        Err(()) => return error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"),
    };
    (StatusCode::NO_CONTENT, cookies).into_response()
}
async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match auth(&state, &headers).await {
        Ok(session) => identity_response(&state, session).await,
        Err(r) => r,
    }
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
            "you cannot grant those project roles",
        ),
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
async fn auth(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<persistence::AppSessionRecord, Response> {
    let token = cookie(headers)
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "authentication required"))?;
    state
        .repo
        .session(&digest(&token))
        .await
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"))?
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "authentication required"))
}
async fn session_response(
    state: &AppState,
    headers: &HeaderMap,
    user: UserId,
    email: String,
) -> Response {
    // Session rotation applies only to V2.  Legacy tokens were never part of the
    // V2 authentication contract and must not affect which session is created.
    for token in named_cookies(headers, COOKIE) {
        if state.repo.delete_session(&digest(&token)).await.is_err() {
            return error(StatusCode::INTERNAL_SERVER_ERROR, "session failure");
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
        return error(StatusCode::INTERNAL_SERVER_ERROR, "session failure");
    };
    let cookies = match session_cookie_headers(&token, state.session_seconds, state.cookie_secure) {
        Ok(value) => value,
        Err(()) => return error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"),
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
async fn identity_response(state: &AppState, session: persistence::AppSessionRecord) -> Response {
    match identity_for_user(state, session.user_id, session.email).await {
        Ok(identity) => Json(identity).into_response(),
        Err(response) => response,
    }
}

async fn identity_for_user(
    state: &AppState,
    user: UserId,
    email: String,
) -> Result<UserWire, Response> {
    // Re-read account type instead of trusting a client value or a historical session
    // claim.  Administrative account changes are therefore visible on the next /me.
    let account_type = state
        .repo
        .account_type(user)
        .await
        .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"))?;
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
    Ok(UserWire {
        id: user.to_string(),
        email,
        account_type: account_type.as_str().to_owned(),
        persona: persona.to_owned(),
        landing_path: if is_admin { "/admin" } else { "/" }.to_owned(),
        capabilities: IdentityCapabilitiesWire {
            can_open_admin: is_admin,
            has_mentor_projects,
        },
    })
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
fn error(status: StatusCode, message: &'static str) -> Response {
    (status, Json(ErrorWire { error: message })).into_response()
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

async fn ui() -> Html<&'static str> {
    Html(include_str!("ui.html"))
}

async fn admin_ui(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let session = match auth(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.repo.account_type(session.user_id).await {
        Ok(account_type) if account_type.is_admin() => {
            Html(include_str!("ui.html")).into_response()
        }
        Ok(_) => error(StatusCode::FORBIDDEN, "administrative access required"),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "session failure"),
    }
}

async fn styles() -> Response {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/styles.css"),
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
}
