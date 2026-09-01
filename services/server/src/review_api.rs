//! V2.1 review HTTP surface with Leader submission and Mentor annotation gates.

use crate::{
    AppState, PrincipalKind, compare_version_manifests, csrf, error, principal_auth, v2_error,
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, patch, post},
};
use blob_store::BlobStore;
use compiler::SyncTexIndex;
use core_types::{LogicalPath, UserId};
use persistence::{GlobalRole, PaperStatus, ReviewThreadInput, V2Error};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use uuid::Uuid;

const THREAD_TYPES: [&str; 2] = ["COMMENT", "SUGGESTION"];
const SEVERITIES: [&str; 4] = ["NOTE", "MINOR", "MAJOR", "BLOCKING"];
const CATEGORIES: [&str; 9] = [
    "WRITING",
    "METHODOLOGY",
    "EVIDENCE",
    "CITATION",
    "FORMATTING",
    "FIGURE",
    "TABLE",
    "EQUATION",
    "SUBMISSION_REQUIREMENT",
];
const MAPPING_STATES: [&str; 5] = [
    "EXACT",
    "APPROXIMATE",
    "PDF_ONLY",
    "SOURCE_CHANGED",
    "SOURCE_DELETED",
];

#[derive(Deserialize)]
struct MessageInput {
    body: String,
}

#[derive(Deserialize)]
struct TransitionInput {
    state: String,
}

#[derive(Deserialize)]
struct ControlsInput {
    severity: String,
    category: String,
    assigned_writer_user_id: Option<String>,
    due_at: Option<String>,
}

#[derive(Deserialize)]
struct SuggestionAcceptInput {
    durable_sequence: u64,
}

#[derive(Deserialize)]
struct SuggestionRejectInput {
    rejection_reason: Option<String>,
}

#[derive(Deserialize)]
struct SynctexInput {
    direction: String,
    file_id: Option<Uuid>,
    line: Option<u32>,
    column: Option<u32>,
    page: Option<u32>,
    x: Option<f64>,
    y: Option<f64>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/static/review.js", get(review_js))
        .route("/static/pdf.min.mjs", get(pdf_js))
        .route("/static/pdf.worker.min.mjs", get(pdf_worker))
        .route("/api/v2/mentor/papers", get(mentor_papers))
        .route("/api/v2/reviews/papers/{paper_id}", get(review_paper))
        .route("/api/v2/reviews/papers/{paper_id}/files", get(review_files))
        .route(
            "/api/v2/reviews/papers/{paper_id}/files/{file_id}",
            get(review_file),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/writers",
            get(review_writers),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/rounds",
            get(review_rounds).post(open_round),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/rounds/{round_id}/approve",
            post(deprecated_approve_round),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/rounds/{round_id}/close",
            post(close_round),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/threads",
            get(review_threads).post(create_thread),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/threads/{thread_id}/messages",
            post(add_message),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/threads/{thread_id}/state",
            post(transition_thread),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/threads/{thread_id}/controls",
            patch(update_controls),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/threads/{thread_id}/suggestion/accept",
            post(accept_suggestion),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/threads/{thread_id}/suggestion/reject",
            post(reject_suggestion),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/activity",
            get(review_activity),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/changes",
            get(changes_since_review),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/synctex",
            post(synctex_mapping),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/report.csv",
            get(report_csv),
        )
        .route(
            "/api/v2/reviews/papers/{paper_id}/report.html",
            get(report_html),
        )
}

async fn review_js() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_bytes!("../static/review.js"),
    )
}

async fn pdf_js() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_bytes!("../static/pdf.min.mjs"),
    )
}

async fn pdf_worker() -> Response {
    static_asset(
        "text/javascript; charset=utf-8",
        include_bytes!("../static/pdf.worker.min.mjs"),
    )
}

fn static_asset(content_type: &'static str, bytes: &'static [u8]) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        Bytes::from_static(bytes),
    )
        .into_response()
}

async fn mentor_papers(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let principal = match role_session(&state, &headers, GlobalRole::Mentor).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.v2.mentor_papers(principal).await {
        Ok(papers) => Json(json!({"schema_version":1,"papers":papers})).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn review_paper(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
) -> Response {
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (paper, role) = match state.v2.review_paper(actor, paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    match state.workspaces.restore(paper.workspace_id).await {
        Ok(workspace) => Json(json!({
            "schema_version":1,"paper":paper,"participant_role":role.as_str(),
            "version":workspace.version().get(),"main_file":workspace.main_file().map(LogicalPath::as_str),
        }))
        .into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"),
    }
}

async fn review_files(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
) -> Response {
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (paper, _) = match state.v2.review_paper(actor, paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    match state.v2.visible_paper_files(paper.workspace_id).await {
        Ok(files) => Json(json!({"schema_version":1,"files":files})).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn review_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, file_id)): Path<(Uuid, Uuid)>,
) -> Response {
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (paper, _) = match state.v2.review_paper(actor, paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let file = match state.v2.paper_file(file_id).await {
        Ok(value) if value.workspace_id == paper.workspace_id && !value.tombstoned => value,
        Ok(_) => return error(StatusCode::NOT_FOUND, "file not found"),
        Err(value) => return v2_error(value),
    };
    match state.v2.file_policy(file_id).await {
        Ok(policy) if policy.visible_to_participants() => {}
        Ok(_) => return error(StatusCode::NOT_FOUND, "file not found"),
        Err(value) => return v2_error(value),
    }
    match state
        .workspaces
        .read_file(paper.workspace_id, &file.path)
        .await
    {
        Ok(bytes) => match String::from_utf8(bytes.to_vec()) {
            Ok(content) => {
                Json(json!({"schema_version":1,"file":file,"content":content})).into_response()
            }
            Err(_) => error(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "binary file is not reviewable",
            ),
        },
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "workspace failure"),
    }
}

async fn review_writers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
) -> Response {
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.v2.review_team_writers(actor, paper_id).await {
        Ok(writers) => Json(json!({"schema_version":1,"writers":writers})).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn review_rounds(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
) -> Response {
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.v2.review_rounds(actor, paper_id).await {
        Ok(rounds) => Json(rounds).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn open_round(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let leader = match role_session(&state, &headers, GlobalRole::Writer).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (paper, _) = match state.v2.review_paper(leader, paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    if !paper.is_team_leader {
        return error(
            StatusCode::FORBIDDEN,
            "Only the Paper Team Leader can send this paper for review.",
        );
    }
    if paper.status != PaperStatus::Active {
        return error(
            StatusCode::CONFLICT,
            "Only an active Paper Team can be sent for review.",
        );
    }
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
    let checkpoint = match state.workspaces.force_snapshot(paper.workspace_id).await {
        Ok(value) => value,
        Err(_) => return error(StatusCode::BAD_REQUEST, "paper must have a main file"),
    };
    let state_hash = checkpoint.snapshot_id().to_hex();
    match state
        .v2
        .open_review_round(leader, paper_id, &state_hash)
        .await
    {
        Ok((round, created)) => (
            if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            },
            Json(round),
        )
            .into_response(),
        Err(V2Error::Conflict {
            entity: "current review PDF",
        }) => error(
            StatusCode::CONFLICT,
            "Compile the current paper before sending it for review.",
        ),
        Err(V2Error::Conflict {
            entity: "active Paper Team review submission",
        }) => error(
            StatusCode::CONFLICT,
            "Only an active Paper Team can be sent for review.",
        ),
        Err(value) => v2_error(value),
    }
}

async fn close_round(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, round_id)): Path<(Uuid, Uuid)>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let leader = match role_session(&state, &headers, GlobalRole::Writer).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .close_review_round(leader, paper_id, round_id)
        .await
    {
        Ok(round) => Json(round).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn deprecated_approve_round(headers: HeaderMap) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    error(
        StatusCode::GONE,
        "Mentor review approval is deprecated; the Team Leader ends review.",
    )
}

async fn review_threads(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
) -> Response {
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.v2.review_threads(actor, paper_id).await {
        Ok(threads) => Json(json!({"schema_version":1,"threads":threads})).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn create_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
    Json(input): Json<ReviewThreadInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let mentor = match role_session(&state, &headers, GlobalRole::Mentor).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(message) = validate_thread(&input) {
        return error(StatusCode::BAD_REQUEST, message);
    }
    match state
        .v2
        .create_review_thread(mentor, paper_id, &input)
        .await
    {
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({"schema_version":1,"thread_id":id})),
        )
            .into_response(),
        Err(V2Error::Conflict {
            entity: "paper review gate",
        }) => error(
            StatusCode::CONFLICT,
            "This paper has not been sent for review.",
        ),
        Err(value) => v2_error(value),
    }
}

async fn add_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, thread_id)): Path<(Uuid, Uuid)>,
    Json(input): Json<MessageInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if input.body.trim().is_empty() || input.body.chars().count() > 20_000 {
        return error(
            StatusCode::BAD_REQUEST,
            "message must contain between 1 and 20000 characters",
        );
    }
    match state
        .v2
        .add_review_message(actor, paper_id, thread_id, &input.body)
        .await
    {
        Ok(id) => (
            StatusCode::CREATED,
            Json(json!({"schema_version":1,"message_id":id})),
        )
            .into_response(),
        Err(value) => v2_error(value),
    }
}

async fn transition_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, thread_id)): Path<(Uuid, Uuid)>,
    Json(input): Json<TransitionInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .transition_review_thread(actor, paper_id, thread_id, &input.state)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(value) => v2_error(value),
    }
}

async fn update_controls(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, thread_id)): Path<(Uuid, Uuid)>,
    Json(input): Json<ControlsInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let mentor = match role_session(&state, &headers, GlobalRole::Mentor).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !SEVERITIES.contains(&input.severity.as_str())
        || !CATEGORIES.contains(&input.category.as_str())
    {
        return error(
            StatusCode::BAD_REQUEST,
            "invalid review severity or category",
        );
    }
    let assigned = match input
        .assigned_writer_user_id
        .as_deref()
        .map(str::parse::<Uuid>)
        .transpose()
    {
        Ok(value) => value.map(UserId::from_uuid),
        Err(_) => return error(StatusCode::BAD_REQUEST, "invalid assigned Writer id"),
    };
    match state
        .v2
        .update_review_controls(
            mentor,
            paper_id,
            thread_id,
            &input.severity,
            &input.category,
            assigned,
            input.due_at.as_deref(),
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(value) => v2_error(value),
    }
}

async fn accept_suggestion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, thread_id)): Path<(Uuid, Uuid)>,
    Json(input): Json<SuggestionAcceptInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let writer = match role_session(&state, &headers, GlobalRole::Writer).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .accept_review_suggestion(writer, paper_id, thread_id, input.durable_sequence)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(value) => v2_error(value),
    }
}

async fn reject_suggestion(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((paper_id, thread_id)): Path<(Uuid, Uuid)>,
    Json(input): Json<SuggestionRejectInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let writer = match role_session(&state, &headers, GlobalRole::Writer).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state
        .v2
        .reject_review_suggestion(
            writer,
            paper_id,
            thread_id,
            input.rejection_reason.as_deref(),
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(value) => v2_error(value),
    }
}

async fn review_activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
) -> Response {
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    match state.v2.review_activity(actor, paper_id).await {
        Ok(events) => Json(json!({"schema_version":1,"events":events})).into_response(),
        Err(value) => v2_error(value),
    }
}

async fn changes_since_review(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
) -> Response {
    let mentor = match role_session(&state, &headers, GlobalRole::Mentor).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let summaries = match state.v2.mentor_papers(mentor).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let Some(summary) = summaries.into_iter().find(|paper| paper.id == paper_id) else {
        return error(StatusCode::NOT_FOUND, "review paper not found");
    };
    let Some(current_id) = summary.current_version_id else {
        return Json(json!({"schema_version":1,"available":false})).into_response();
    };
    let rounds = match state.v2.review_rounds(mentor, paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let baseline_id = rounds
        .rounds
        .first()
        .and_then(|round| round["baseline_version_id"].as_str())
        .and_then(|value| Uuid::parse_str(value).ok());
    let Some(baseline_id) = baseline_id else {
        return Json(json!({"schema_version":1,"available":false})).into_response();
    };
    let from = match state.v2.paper_version(mentor, paper_id, baseline_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let to = match state.v2.paper_version(mentor, paper_id, current_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    match compare_version_manifests(&state, &from.manifest, &to.manifest).await {
        Ok(changes) => Json(json!({"schema_version":1,"available":true,"baseline_version_id":baseline_id,"current_version_id":current_id,"changes":changes})).into_response(),
        Err(response) => response,
    }
}

async fn synctex_mapping(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
    Json(input): Json<SynctexInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (paper, _) = match state.v2.review_paper(actor, paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let artifact = match state
        .v2
        .current_v2_artifact(actor, paper_id, "synctex")
        .await
    {
        Ok(value) => value,
        Err(_) => return Json(mapping_fallback("PDF_ONLY", None, None)).into_response(),
    };
    let bytes = match state.blobs.get(artifact.blob_hash).await {
        Ok(value) => value,
        Err(_) => return Json(mapping_fallback("PDF_ONLY", None, None)).into_response(),
    };
    let index = match SyncTexIndex::from_gzip(&bytes) {
        Ok(value) => value,
        Err(_) => return Json(mapping_fallback("PDF_ONLY", None, None)).into_response(),
    };
    let files = match state.v2.visible_paper_files(paper.workspace_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let build = match state.v2.v2_build_view(actor, paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let location = match input.direction.as_str() {
        "FORWARD" => {
            let Some(file_id) = input.file_id else {
                return error(StatusCode::BAD_REQUEST, "FORWARD mapping requires file_id");
            };
            let Some(file) = files.iter().find(|file| file.file_id == file_id) else {
                return error(StatusCode::NOT_FOUND, "source file not found");
            };
            index.forward(
                file.path.as_str(),
                input.line.unwrap_or(1),
                input.column.unwrap_or(0),
            )
        }
        "INVERSE" => index.inverse(
            input.page.unwrap_or(0),
            input.x.unwrap_or(f64::NAN),
            input.y.unwrap_or(f64::NAN),
        ),
        _ => {
            return error(
                StatusCode::BAD_REQUEST,
                "direction must be FORWARD or INVERSE",
            );
        }
    };
    let Some(location) = location else {
        return Json(mapping_fallback(
            "PDF_ONLY",
            build.current_build_id,
            Some(artifact.artifact_id),
        ))
        .into_response();
    };
    let mapped_file = files
        .iter()
        .find(|file| path_matches(&location.source_path, file.path.as_str()));
    if input.direction == "INVERSE" && mapped_file.is_none() {
        return Json(mapping_fallback(
            "PDF_ONLY",
            build.current_build_id,
            Some(artifact.artifact_id),
        ))
        .into_response();
    }
    Json(json!({
        "schema_version":1,"mapping_status":if location.exact {"EXACT"} else {"APPROXIMATE"},
        "build_id":build.current_build_id,"artifact_id":artifact.artifact_id,"page":location.page,
        "x":location.x,"y":location.y,"width":location.width,"height":location.height,
        "mapped_file_id":mapped_file.map(|file|file.file_id),"mapped_path":mapped_file.map(|file|file.path.as_str()),
        "mapped_line":location.line,"mapped_column":location.column,
    })).into_response()
}

async fn report_csv(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
) -> Response {
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let threads = match state.v2.review_threads(actor, paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let mut csv = String::from(
        "round,thread_type,status,severity,category,assigned_writer,due_date,source_file,source_context,pdf_page,initial_message,created_at,resolved_at\n",
    );
    for thread in threads {
        let source = &thread["source_anchor"];
        let message = thread["messages"]
            .as_array()
            .and_then(|messages| messages.first())
            .and_then(|message| message["body"].as_str())
            .unwrap_or("");
        let fields = [
            thread["round_number"].to_string(),
            text(&thread, "thread_type"),
            text(&thread, "state"),
            text(&thread, "severity"),
            text(&thread, "category"),
            text(&thread, "assigned_writer_email"),
            text(&thread, "due_at"),
            text(source, "path"),
            text(source, "quoted_text"),
            thread["pdf_anchor"]["page"].to_string(),
            message.to_owned(),
            text(&thread, "created_at"),
            text(&thread, "resolved_at"),
        ];
        csv.push_str(
            &fields
                .into_iter()
                .map(|value| csv_field(&value))
                .collect::<Vec<_>>()
                .join(","),
        );
        csv.push('\n');
    }
    (
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"review-report.csv\"",
            ),
        ],
        csv,
    )
        .into_response()
}

async fn report_html(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
) -> Response {
    let actor = match participant_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (paper, _) = match state.v2.review_paper(actor, paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let threads = match state.v2.review_threads(actor, paper_id).await {
        Ok(value) => value,
        Err(value) => return v2_error(value),
    };
    let mut rows = String::new();
    for thread in threads {
        let message = thread["messages"]
            .as_array()
            .and_then(|messages| messages.first())
            .and_then(|message| message["body"].as_str())
            .unwrap_or("");
        write!(
            rows,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            html(&thread["round_number"].to_string()),
            html(&text(&thread, "thread_type")),
            html(&text(&thread, "state")),
            html(&text(&thread, "severity")),
            html(&text(&thread, "category")),
            html(message)
        )
        .expect("writing HTML into a String cannot fail");
    }
    Html(format!("<!doctype html><html><head><meta charset=utf-8><title>Review report — {0}</title><style>body{{font:12pt system-ui;margin:2cm}}table{{width:100%;border-collapse:collapse}}th,td{{padding:6px;border:1px solid #aaa;text-align:left;vertical-align:top}}@media print{{button{{display:none}}}}</style></head><body><button onclick=\"print()\">Print / Save as PDF</button><h1>Review report — {0}</h1><p>Print-ready report for browser Print → Save as PDF.</p><table><thead><tr><th>Round</th><th>Type</th><th>Status</th><th>Severity</th><th>Category</th><th>Initial message</th></tr></thead><tbody>{1}</tbody></table></body></html>",html(&paper.name),rows)).into_response()
}

async fn participant_session(state: &AppState, headers: &HeaderMap) -> Result<UserId, Response> {
    let principal = principal_auth(state, headers).await?;
    if !matches!(
        principal.kind,
        PrincipalKind::V2(GlobalRole::Writer | GlobalRole::Mentor)
    ) {
        return Err(error(StatusCode::FORBIDDEN, "review participant required"));
    }
    Ok(principal.user_id())
}

async fn role_session(
    state: &AppState,
    headers: &HeaderMap,
    required: GlobalRole,
) -> Result<UserId, Response> {
    let principal = principal_auth(state, headers).await?;
    if !matches!(principal.kind, PrincipalKind::V2(role) if role == required) {
        return Err(error(StatusCode::FORBIDDEN, "exclusive V2 role required"));
    }
    Ok(principal.user_id())
}

fn validate_thread(input: &ReviewThreadInput) -> Result<(), &'static str> {
    if !THREAD_TYPES.contains(&input.thread_type.as_str())
        || input.severity != "NOTE"
        || input.category != "WRITING"
    {
        return Err("new reviews use COMMENT or SUGGESTION with default metadata");
    }
    if input.message.trim().is_empty() || input.message.chars().count() > 20_000 {
        return Err("message must contain between 1 and 20000 characters");
    }
    if input.thread_type == "SUGGESTION"
        && (input.source_anchor.is_none() || input.pdf_anchor.is_some())
    {
        return Err("suggestion requires one source anchor");
    }
    if input.thread_type == "SUGGESTION"
        && input
            .suggested_replacement
            .as_deref()
            .is_some_and(|value| value.trim().is_empty() || value.chars().count() > 20_000)
    {
        return Err("suggestion text must contain between 1 and 20000 characters");
    }
    if input.thread_type == "COMMENT" && input.suggested_replacement.is_some() {
        return Err("suggestion text is valid only for a suggestion");
    }
    if input.assigned_writer_user_id.is_some() || input.due_at.is_some() {
        return Err("new comments and suggestions do not accept assignment or due dates");
    }
    if input.source_anchor.is_none() == input.pdf_anchor.is_none() {
        return Err("review thread requires exactly one source or PDF anchor");
    }
    if let Some(anchor) = &input.source_anchor {
        if !valid_hash(&anchor.context_hash)
            || anchor.encoded_relative_start.is_none()
            || anchor.encoded_relative_start.is_some() != anchor.encoded_relative_end.is_some()
            || anchor.quoted_text.is_empty()
        {
            return Err("invalid source anchor");
        }
    }
    if let Some(anchor) = &input.pdf_anchor {
        if anchor.page < 1
            || !MAPPING_STATES.contains(&anchor.mapping_status.as_str())
            || !valid_rectangles(&anchor.normalized_rectangles)
        {
            return Err("invalid PDF anchor");
        }
    }
    Ok(())
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}
fn valid_rectangles(value: &Value) -> bool {
    let Some(rectangles) = value.as_array() else {
        return false;
    };
    !rectangles.is_empty()
        && rectangles.len() <= 64
        && rectangles.iter().all(|rectangle| {
            let values =
                ["x", "y", "width", "height"].map(|key| rectangle.get(key).and_then(Value::as_f64));
            let [Some(x), Some(y), Some(width), Some(height)] = values else {
                return false;
            };
            [x, y, width, height].into_iter().all(f64::is_finite)
                && x >= 0.0
                && y >= 0.0
                && width > 0.0
                && height > 0.0
                && x + width <= 1.000_001
                && y + height <= 1.000_001
        })
}
fn path_matches(artifact: &str, path: &str) -> bool {
    artifact == path
        || artifact
            .strip_suffix(path)
            .is_some_and(|prefix| prefix.ends_with('/'))
}
fn mapping_fallback(status: &str, build_id: Option<Uuid>, artifact_id: Option<Uuid>) -> Value {
    json!({"schema_version":1,"mapping_status":status,"build_id":build_id,"artifact_id":artifact_id})
}
fn text(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned()
}
fn csv_field(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[allow(
    dead_code,
    reason = "kept beside validation to make browser/server context hashes auditable"
)]
fn context_hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_comment_defaults_hidden_legacy_metadata() {
        let input: ReviewThreadInput = serde_json::from_value(json!({
            "thread_type": "COMMENT",
            "message": "Clarify this sentence.",
            "source_anchor": {
                "file_id": Uuid::new_v4(),
                "encoded_relative_start": [1],
                "encoded_relative_end": [2],
                "quoted_text": "sentence",
                "context_hash": "0".repeat(64),
                "source_sequence": 1,
                "source_version_id": null,
                "document_epoch": 1
            }
        }))
        .expect("simple comment input should deserialize");

        assert_eq!(input.severity, "NOTE");
        assert_eq!(input.category, "WRITING");
        assert!(input.assigned_writer_user_id.is_none());
        assert!(input.due_at.is_none());
        assert_eq!(validate_thread(&input), Ok(()));
    }

    #[test]
    fn new_review_creation_rejects_legacy_types_and_controls() {
        let legacy: ReviewThreadInput = serde_json::from_value(json!({
            "thread_type": "QUESTION",
            "message": "Why?",
            "severity": "NOTE",
            "category": "WRITING",
            "pdf_anchor": {
                "page": 1,
                "normalized_rectangles": [{"x":0.1,"y":0.1,"width":0.2,"height":0.1}],
                "mapping_status": "PDF_ONLY",
                "mapped_file_id": null,
                "mapped_line": null,
                "mapped_column": null
            }
        }))
        .expect("legacy review input should deserialize for rejection testing");
        assert!(validate_thread(&legacy).is_err());

        let mut controlled = legacy;
        controlled.thread_type = "COMMENT".to_owned();
        controlled.severity = "MAJOR".to_owned();
        assert!(validate_thread(&controlled).is_err());
    }
}
