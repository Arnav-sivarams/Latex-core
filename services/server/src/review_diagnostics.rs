//! Bounded client reports for the existing operator Runtime Logs facility.
use crate::{AppState, csrf, error, review_api::role_session};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use persistence::GlobalRole;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use uuid::Uuid;

#[derive(Clone, Copy, Deserialize, Serialize, Debug)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ClientErrorCode {
    SourceAnchorFailed,
    PublicationUnconfirmed,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClientErrorInput {
    schema_version: u8,
    code: ClientErrorCode,
    round_id: Option<Uuid>,
}

#[derive(Default)]
struct ReportLimit {
    windows: BTreeMap<Uuid, (Instant, u8)>,
}
impl ReportLimit {
    fn admit(&mut self, actor: Uuid, now: Instant) -> bool {
        self.windows
            .retain(|_, (start, _)| now.duration_since(*start) < Duration::from_secs(60));
        if !self.windows.contains_key(&actor) && self.windows.len() >= 1024 {
            return false;
        }
        let (_, count) = self.windows.entry(actor).or_insert((now, 0));
        if *count >= 5 {
            return false;
        }
        *count += 1;
        true
    }
}

static LIMIT: std::sync::OnceLock<Mutex<ReportLimit>> = std::sync::OnceLock::new();

pub async fn client_error(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(paper_id): Path<Uuid>,
    Json(input): Json<ClientErrorInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let actor = match role_session(&state, &headers, GlobalRole::Mentor).await {
        Ok(actor) => actor,
        Err(response) => return response,
    };
    if let Err(value) = state.v2.review_paper(actor, paper_id).await {
        return crate::v2_error(value);
    }
    if input.schema_version != 1 {
        return error(
            StatusCode::BAD_REQUEST,
            "Unsupported diagnostic schema version.",
        );
    }
    let allowed = match LIMIT
        .get_or_init(|| Mutex::new(ReportLimit::default()))
        .lock()
    {
        Ok(mut limiter) => limiter.admit(*actor.as_uuid(), Instant::now()),
        Err(_) => {
            return error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Client diagnostics unavailable.",
            );
        }
    };
    if !allowed {
        return error(
            StatusCode::TOO_MANY_REQUESTS,
            "Client diagnostic report limit reached.",
        );
    }
    let request_id = Uuid::new_v4();
    tracing::warn!(event = "CLIENT_REPORTED_ERROR", %request_id, %actor, %paper_id, round_id = ?input.round_id, error_code = ?input.code, verified_publication = false);
    StatusCode::NO_CONTENT.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_are_bounded_per_actor_and_expire() {
        let mut limit = ReportLimit::default();
        let now = Instant::now();
        let actor = Uuid::from_u128(1);
        for _ in 0..5 {
            assert!(limit.admit(actor, now));
        }
        assert!(!limit.admit(actor, now));
        assert!(limit.admit(Uuid::from_u128(2), now));
        assert!(limit.admit(actor, now + Duration::from_secs(60)));
        for id in 2..=1024 {
            assert!(limit.admit(Uuid::from_u128(id), now + Duration::from_secs(60)));
        }
        assert!(!limit.admit(Uuid::from_u128(1025), now + Duration::from_secs(60)));
        assert_eq!(limit.windows.len(), 1024);
    }
    #[test]
    fn diagnostics_accept_only_codes_and_identifiers() {
        assert!(
            serde_json::from_str::<ClientErrorInput>(
                r#"{"schema_version":1,"code":"SOURCE_ANCHOR_FAILED","round_id":null}"#
            )
            .is_ok()
        );
        assert!(
            serde_json::from_str::<ClientErrorInput>(
                r#"{"schema_version":1,"code":"SOURCE_ANCHOR_FAILED","message":"private source"}"#
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<ClientErrorInput>(
                r#"{"schema_version":1,"code":"arbitrary private text"}"#
            )
            .is_err()
        );
    }
}
