//! Versioned, allowlisted institutional server-to-server read API.

use super::{AppState, admin_session, csrf};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use blob_store::BlobStore;
use persistence::{IntegrationError, IntegrationPrincipal, IntegrationRepository, parse_blob_hash};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use uuid::Uuid;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 100;

#[derive(Deserialize)]
struct ClientInput {
    name: String,
    scopes: Vec<String>,
    #[serde(default)]
    institution_wide: bool,
    #[serde(default)]
    report_ids: Vec<Uuid>,
    expires_at: Option<String>,
}

#[derive(Deserialize)]
struct PageQuery {
    limit: Option<i64>,
    cursor: Option<String>,
}

#[derive(Deserialize)]
struct ReportQuery {
    limit: Option<i64>,
    cursor: Option<String>,
    programme: Option<String>,
    academic_year: Option<String>,
    semester: Option<String>,
    state: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/api/admin/integration/v1/clients",
            get(admin_clients).post(admin_create_client),
        )
        .route(
            "/api/admin/integration/v1/clients/{client_id}/revoke",
            post(admin_revoke_client),
        )
        .route(
            "/api/admin/integration/v1/clients/{client_id}/rotate",
            post(admin_rotate_client),
        )
        .route("/api/integration/v1/reports", get(reports))
        .route("/api/integration/v1/reports/{report_id}", get(report))
        .route(
            "/api/integration/v1/reports/{report_id}/front-matter",
            get(front_matter),
        )
        .route(
            "/api/integration/v1/reports/{report_id}/versions",
            get(versions),
        )
        .route(
            "/api/integration/v1/reports/{report_id}/versions/{version_id}/files",
            get(version_files),
        )
        .route(
            "/api/integration/v1/reports/{report_id}/versions/{version_id}/files/{file_id}/content",
            get(version_file_content),
        )
        .route(
            "/api/integration/v1/reports/{report_id}/builds/{build_id}/pdf",
            get(pdf),
        )
        .route(
            "/api/integration/v1/reports/{report_id}/builds",
            get(builds),
        )
        .route(
            "/api/integration/v1/reports/{report_id}/reviews/published",
            get(published_reviews),
        )
        .route("/api/integration/v1/directory/{resource}", get(directory))
}

async fn admin_clients(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let admin = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match IntegrationRepository::new(state.database.clone())
        .list_clients(admin.user_id())
        .await
    {
        Ok(clients) => Json(json!({"schema_version":1,"clients":clients})).into_response(),
        Err(error) => integration_error(error),
    }
}

async fn admin_create_client(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<ClientInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let admin = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (secret, prefix, hash) = new_token();
    match IntegrationRepository::new(state.database.clone())
        .create_client(
            admin.user_id(),
            &input.name,
            &prefix,
            &hash,
            &input.scopes,
            input.institution_wide,
            &input.report_ids,
            input.expires_at.as_deref(),
        )
        .await
    {
        Ok(client) => (
            StatusCode::CREATED,
            Json(
                json!({"schema_version":1,"client":client,"secret":secret,"secret_display":"once"}),
            ),
        )
            .into_response(),
        Err(error) => integration_error(error),
    }
}

async fn admin_revoke_client(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(client_id): Path<Uuid>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let admin = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match IntegrationRepository::new(state.database.clone())
        .revoke_client(admin.user_id(), client_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => integration_error(error),
    }
}

async fn admin_rotate_client(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(client_id): Path<Uuid>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let admin = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (secret, prefix, hash) = new_token();
    match IntegrationRepository::new(state.database.clone())
        .rotate_client(admin.user_id(), client_id, &prefix, &hash)
        .await
    {
        Ok(client) => Json(
            json!({"schema_version":1,"client":client,"secret":secret,"secret_display":"once"}),
        )
        .into_response(),
        Err(error) => integration_error(error),
    }
}

async fn reports(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReportQuery>,
) -> Response {
    let principal = match machine_principal(&state, &headers, "reports.read").await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (limit, cursor) = match uuid_page(query.limit, query.cursor.as_deref()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match IntegrationRepository::new(state.database.clone())
        .reports(
            &principal,
            cursor,
            limit + 1,
            query.programme.as_deref(),
            query.academic_year.as_deref(),
            query.semester.as_deref(),
            query.state.as_deref(),
        )
        .await
    {
        Ok(values) => page(values, limit, "id"),
        Err(error) => integration_error(error),
    }
}

async fn report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(report_id): Path<Uuid>,
) -> Response {
    let principal = match report_principal(&state, &headers, "reports.read", report_id).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let contacts = principal.has_scope("institution.contacts.read");
    match IntegrationRepository::new(state.database.clone())
        .report(report_id, contacts)
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error) => integration_error(error),
    }
}

async fn front_matter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(report_id): Path<Uuid>,
) -> Response {
    if let Err(response) = report_principal(&state, &headers, "reports.read", report_id).await {
        return response;
    }
    match state.front_matter.archive_metadata(report_id).await {
        Ok(value) => {
            Json(json!({"schema_version":1,"report_id":report_id,"current":value})).into_response()
        }
        Err(_) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "persistence_error",
            "Front Matter metadata is unavailable",
        ),
    }
}

async fn versions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(report_id): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Response {
    if let Err(response) = report_principal(&state, &headers, "reports.files.read", report_id).await
    {
        return response;
    }
    let (limit, cursor) = match uuid_page(query.limit, query.cursor.as_deref()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match IntegrationRepository::new(state.database.clone())
        .versions(report_id, cursor, limit + 1)
        .await
    {
        Ok(values) => page(values, limit, "id"),
        Err(error) => integration_error(error),
    }
}

async fn version_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((report_id, version_id)): Path<(Uuid, Uuid)>,
) -> Response {
    if let Err(response) = report_principal(&state, &headers, "reports.files.read", report_id).await
    {
        return response;
    }
    match IntegrationRepository::new(state.database.clone())
        .version(report_id, version_id)
        .await
    {
        Ok(version) => Json(json!({"schema_version":1,"report_id":report_id,"version_id":version_id,"snapshot_id":version["snapshot_id"],"files":permitted_files(&version)})).into_response(),
        Err(error) => integration_error(error),
    }
}

async fn version_file_content(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((report_id, version_id, file_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Response {
    if let Err(response) = report_principal(&state, &headers, "reports.files.read", report_id).await
    {
        return response;
    }
    let version = match IntegrationRepository::new(state.database.clone())
        .version(report_id, version_id)
        .await
    {
        Ok(value) => value,
        Err(error) => return integration_error(error),
    };
    let Some(file) = permitted_files(&version)
        .into_iter()
        .find(|value| value["file_id"] == json!(file_id))
    else {
        return api_error(StatusCode::NOT_FOUND, "not_found", "version file not found");
    };
    let Some(hash) = file["sha256"].as_str() else {
        return api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "persistence_error",
            "invalid file identity",
        );
    };
    let hash = match parse_blob_hash(hash) {
        Ok(value) => value,
        Err(error) => return integration_error(error),
    };
    match state.blobs.get(hash).await {
        Ok(bytes) => binary(
            bytes,
            media_type(file["path"].as_str().unwrap_or("")),
            &hash.to_hex(),
            None,
        ),
        Err(_) => api_error(
            StatusCode::NOT_FOUND,
            "not_available",
            "version file content is unavailable",
        ),
    }
}

async fn pdf(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((report_id, build_id)): Path<(Uuid, Uuid)>,
) -> Response {
    if let Err(response) = report_principal(&state, &headers, "reports.pdf.read", report_id).await {
        return response;
    }
    let record = match IntegrationRepository::new(state.database.clone())
        .pdf(report_id, build_id)
        .await
    {
        Ok(value) => value,
        Err(IntegrationError::NotFound) => {
            return api_error(
                StatusCode::NOT_FOUND,
                "not_compiled",
                "No successful PDF exists for this report/build identity",
            );
        }
        Err(error) => return integration_error(error),
    };
    let hash = match record["blob_hash"]
        .as_str()
        .and_then(|value| parse_blob_hash(value).ok())
    {
        Some(value) => value,
        None => {
            return api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "persistence_error",
                "invalid PDF identity",
            );
        }
    };
    match state.blobs.get(hash).await {
        Ok(bytes) => binary(
            bytes,
            "application/pdf",
            &hash.to_hex(),
            record["version_id"].as_str(),
        ),
        Err(_) => api_error(
            StatusCode::NOT_FOUND,
            "not_available",
            "PDF blob is unavailable",
        ),
    }
}

async fn builds(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(report_id): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Response {
    if let Err(response) = report_principal(&state, &headers, "reports.pdf.read", report_id).await {
        return response;
    }
    let (limit, cursor) = match uuid_page(query.limit, query.cursor.as_deref()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match IntegrationRepository::new(state.database.clone())
        .builds(report_id, cursor, limit + 1)
        .await
    {
        Ok(values) => page(values, limit, "id"),
        Err(error) => integration_error(error),
    }
}

async fn directory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(resource): Path<String>,
    Query(query): Query<PageQuery>,
) -> Response {
    let principal = match machine_principal(&state, &headers, "institution.directory.read").await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !principal.institution_wide {
        return api_error(
            StatusCode::FORBIDDEN,
            "coverage_denied",
            "institution-wide directory coverage was not granted",
        );
    }
    let limit = match bounded_limit(query.limit) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let contacts = principal.has_scope("institution.contacts.read");
    match IntegrationRepository::new(state.database.clone())
        .directory(&resource, query.cursor.as_deref(), limit + 1, contacts)
        .await
    {
        Ok(values) => page(values, limit, "id"),
        Err(error) => integration_error(error),
    }
}

async fn published_reviews(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(report_id): Path<Uuid>,
    Query(query): Query<PageQuery>,
) -> Response {
    if let Err(response) =
        report_principal(&state, &headers, "reviews.published.read", report_id).await
    {
        return response;
    }
    let (limit, cursor) = match uuid_page(query.limit, query.cursor.as_deref()) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match IntegrationRepository::new(state.database.clone())
        .published_reviews(report_id, cursor, limit + 1)
        .await
    {
        Ok(values) => page(values, limit, "id"),
        Err(error) => integration_error(error),
    }
}

async fn machine_principal(
    state: &AppState,
    headers: &HeaderMap,
    scope: &str,
) -> Result<IntegrationPrincipal, Response> {
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            api_error(
                StatusCode::UNAUTHORIZED,
                "authentication_required",
                "Bearer authentication is required",
            )
        })?;
    let token = authorization
        .strip_prefix("Bearer ")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            api_error(
                StatusCode::UNAUTHORIZED,
                "invalid_credential",
                "Authorization must use a Bearer credential",
            )
        })?;
    let hash = Sha256::digest(token.as_bytes());
    let principal = IntegrationRepository::new(state.database.clone())
        .authenticate(hash.as_slice())
        .await
        .map_err(integration_error)?;
    if !principal.has_scope(scope) {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "scope_denied",
            format!("required scope: {scope}"),
        ));
    }
    IntegrationRepository::new(state.database.clone())
        .admit_read(principal.client_id, scope)
        .await
        .map_err(integration_error)?;
    Ok(principal)
}

async fn report_principal(
    state: &AppState,
    headers: &HeaderMap,
    scope: &str,
    report_id: Uuid,
) -> Result<IntegrationPrincipal, Response> {
    let principal = machine_principal(state, headers, scope).await?;
    if !principal.allows_report(report_id) {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "report not found",
        ));
    }
    if !IntegrationRepository::new(state.database.clone())
        .report_exists(report_id)
        .await
        .map_err(integration_error)?
    {
        return Err(api_error(
            StatusCode::NOT_FOUND,
            "not_found",
            "report not found",
        ));
    }
    Ok(principal)
}

fn new_token() -> (String, String, Vec<u8>) {
    let mut random = [0_u8; 32];
    rand::rng().fill_bytes(&mut random);
    let secret = format!("lcint_{}", hex::encode(random));
    let prefix = secret.chars().take(16).collect::<String>();
    let hash = Sha256::digest(secret.as_bytes()).to_vec();
    (secret, prefix, hash)
}

fn bounded_limit(limit: Option<i64>) -> Result<i64, Response> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if (1..=MAX_LIMIT).contains(&limit) {
        Ok(limit)
    } else {
        Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid_limit",
            "limit must be between 1 and 100",
        ))
    }
}

fn uuid_page(limit: Option<i64>, cursor: Option<&str>) -> Result<(i64, Option<Uuid>), Response> {
    let limit = bounded_limit(limit)?;
    let cursor = cursor.map(Uuid::parse_str).transpose().map_err(|_| {
        api_error(
            StatusCode::BAD_REQUEST,
            "invalid_cursor",
            "cursor is malformed",
        )
    })?;
    Ok((limit, cursor))
}

fn page(mut values: Vec<Value>, limit: i64, key: &str) -> Response {
    let has_more = values.len() > usize::try_from(limit).unwrap_or_default();
    values.truncate(usize::try_from(limit).unwrap_or_default());
    let next_cursor = has_more
        .then(|| {
            values
                .last()
                .and_then(|value| value[key].as_str())
                .map(str::to_owned)
        })
        .flatten();
    Json(json!({"schema_version":1,"data":values,"page":{"limit":limit,"next_cursor":next_cursor}}))
        .into_response()
}

fn permitted_files(version: &Value) -> Vec<Value> {
    let identities = version["file_identities"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let policies = version["file_policies"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| {
            Some((
                value["file_id"].as_str()?.to_owned(),
                value["policy"].as_str()?.to_owned(),
            ))
        })
        .collect::<BTreeMap<_, _>>();
    identities.into_iter().filter_map(|identity| {
        let file_id = identity["file_id"].as_str()?;
        let path = identity["path"].as_str()?;
        if policies.get(file_id).is_some_and(|policy| policy == "HIDDEN_SYSTEM") { return None; }
        let entry = version["files"].get(path)?;
        Some(json!({"file_id":file_id,"path":path,"sha256":entry["blob_hash"],"size_bytes":entry["size_bytes"],"content_url":format!("/api/integration/v1/reports/{}/versions/{}/files/{file_id}/content",version.get("report_id").and_then(Value::as_str).unwrap_or("REPORT_ID"),version["id"].as_str().unwrap_or("VERSION_ID"))}))
    }).collect()
}

fn media_type(path: &str) -> &'static str {
    match path
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("tex") => "application/x-tex",
        Some("bib") => "application/x-bibtex",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("pdf") => "application/pdf",
        _ => "application/octet-stream",
    }
}

fn binary(
    bytes: bytes::Bytes,
    content_type: &'static str,
    hash: &str,
    version_id: Option<&str>,
) -> Response {
    let mut response = bytes.into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Ok(value) = HeaderValue::from_str(&format!("\"{hash}\"")) {
        response.headers_mut().insert(header::ETAG, value);
    }
    if let Some(version_id) = version_id.and_then(|value| HeaderValue::from_str(value).ok()) {
        response
            .headers_mut()
            .insert("x-latex-core-version-id", version_id);
    }
    response
}

fn integration_error(error: IntegrationError) -> Response {
    match error {
        IntegrationError::NotFound => {
            api_error(StatusCode::NOT_FOUND, "not_found", "resource not found")
        }
        IntegrationError::Forbidden => api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_credential",
            "credential is invalid, expired, or revoked",
        ),
        IntegrationError::RateLimited => api_error(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "integration client exceeded 120 requests per minute",
        ),
        IntegrationError::Invalid(message) => {
            api_error(StatusCode::BAD_REQUEST, "invalid_request", message)
        }
        IntegrationError::Database(_) | IntegrationError::Integrity(_) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "persistence_error",
            "integration persistence failed",
        ),
    }
}

fn api_error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({"schema_version":1,"code":code,"error":message.into()})),
    )
        .into_response()
}
