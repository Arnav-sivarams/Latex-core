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
    AppError, AppRepository, Database, DatabaseConfig, EnqueueCompileJobV1, PostgresCompileQueue,
    QueueLimits,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::{env, net::SocketAddr, str::FromStr, sync::Arc, time::Duration};
use tower_http::{
    limit::RequestBodyLimitLayer, set_header::SetResponseHeaderLayer, trace::TraceLayer,
};
use workspace_model::WorkspaceService;

const COOKIE: &str = "latex_core_session";
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
struct ImportQuery {
    name: String,
}
#[derive(Deserialize)]
struct SetMain {
    path: String,
    version: u64,
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
}
#[derive(Serialize)]
struct ProjectWire {
    id: String,
    name: String,
    version: u64,
    main_file: Option<String>,
    files: Vec<FileWire>,
}
#[derive(Serialize)]
struct FileWire {
    path: String,
    size_bytes: u64,
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
            get(file).put(put_file).delete(delete_file),
        )
        .route("/api/projects/{id}/main", post(set_main))
        .route("/api/projects/{id}/compile", post(submit_compile))
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
        Ok(user) => session_response(&state, user.user_id, user.email).await,
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
    session_response(&state, user.user_id, user.email).await
}
async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(r) = csrf(&headers) {
        return r;
    };
    if let Some(token) = cookie(&headers) {
        let _ = state.repo.delete_session(&digest(&token)).await;
    }
    let value = format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    (StatusCode::NO_CONTENT, [(header::SET_COOKIE, value)]).into_response()
}
async fn me(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match auth(&state, &headers).await {
        Ok(session) => Json(UserWire {
            id: session.user_id.to_string(),
            email: session.email,
        })
        .into_response(),
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
    if auth(&state, &headers).await.is_err() {
        return error(StatusCode::UNAUTHORIZED, "authentication required");
    }
    match state.repo.list_templates().await {
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
    if state
        .repo
        .assert_project_owner(s.user_id, id)
        .await
        .is_err()
    {
        return error(StatusCode::NOT_FOUND, "not found");
    };
    match state.workspaces.restore(id).await {
        Ok(v) => Json(
            v.files()
                .iter()
                .map(|(p, f)| FileWire {
                    path: p.as_str().to_owned(),
                    size_bytes: f.size_bytes(),
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
    if state
        .repo
        .assert_project_owner(s.user_id, id)
        .await
        .is_err()
    {
        return error(StatusCode::NOT_FOUND, "not found");
    };
    match state.workspaces.read_file(id, &path).await {
        Ok(b) => {
            let mut response = b.into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/plain; charset=utf-8"),
            );
            let etag = workspace_etag(&state, id).await;
            match HeaderValue::from_str(&etag) {
                Ok(value) => {
                    response.headers_mut().insert(header::ETAG, value);
                    response
                }
                Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "header failure"),
            }
        }
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
    if state
        .repo
        .assert_project_owner(s.user_id, id)
        .await
        .is_err()
    {
        return error(StatusCode::NOT_FOUND, "not found");
    };
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
    if state
        .repo
        .assert_project_owner(s.user_id, id)
        .await
        .is_err()
    {
        return error(StatusCode::NOT_FOUND, "not found");
    };
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
        Err(_) => error(StatusCode::BAD_REQUEST, "file delete failed"),
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
    if state
        .repo
        .assert_project_owner(s.user_id, id)
        .await
        .is_err()
    {
        return error(StatusCode::NOT_FOUND, "not found");
    };
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
    if state
        .repo
        .assert_project_owner(s.user_id, id)
        .await
        .is_err()
    {
        return error(StatusCode::NOT_FOUND, "not found");
    };
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

async fn project_response(state: &AppState, user: UserId, id: WorkspaceId) -> Response {
    if state.repo.assert_project_owner(user, id).await.is_err() {
        return error(StatusCode::NOT_FOUND, "not found");
    };
    match state.workspaces.restore(id).await {
        Ok(v) => {
            let p = match state.repo.project(user, id).await {
                Ok(v) => v,
                Err(_) => return error(StatusCode::NOT_FOUND, "not found"),
            };
            Json(ProjectWire {
                id: id.to_string(),
                name: p.name,
                version: v.version().get(),
                main_file: v.main_file().map(|p| p.as_str().to_owned()),
                files: v
                    .files()
                    .iter()
                    .map(|(p, f)| FileWire {
                        path: p.as_str().to_owned(),
                        size_bytes: f.size_bytes(),
                    })
                    .collect(),
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
async fn session_response(state: &AppState, user: UserId, email: String) -> Response {
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
    let secure = if state.cookie_secure { "; Secure" } else { "" };
    let cookie = format!(
        "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{secure}",
        state.session_seconds
    );
    (
        StatusCode::CREATED,
        [(header::SET_COOKIE, cookie)],
        Json(UserWire {
            id: user.to_string(),
            email,
        }),
    )
        .into_response()
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
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(&format!("{COOKIE}=")).map(str::to_owned))
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

#[cfg(test)]
#[allow(clippy::expect_used, reason = "unit assertion fixture")]
mod tests {
    use super::*;

    #[test]
    fn session_cookie_parser_ignores_unrelated_cookie_values() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("other=x; latex_core_session=token; x=y"),
        );
        assert_eq!(cookie(&headers).as_deref(), Some("token"));
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
    }
}
