//! Support contact configuration uses the existing institution settings singleton.
use super::*;

#[derive(Deserialize, Serialize)]
pub struct SupportInput {
    pub support_email: Option<String>,
}

pub fn valid_email(email: &str) -> bool {
    let Some((local, domain)) = email.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.split('.').count() >= 2
        && domain.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                && !part.starts_with('-')
                && !part.ends_with('-')
        })
        && email.len() <= 254
        && !email.chars().any(char::is_whitespace)
        && email
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".!#$%&'*+-/=?^_`{|}~@".contains(&byte))
        && !domain.contains('@')
}

pub async fn read(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = principal_auth(&state, &headers).await {
        return response;
    }
    match state.institution.support_email().await {
        Ok(value) => {
            Json(serde_json::json!({"schema_version":1,"support_email":value})).into_response()
        }
        Err(value) => institution_error(value),
    }
}

pub async fn save(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<SupportInput>,
) -> Response {
    if let Err(response) = csrf(&headers) {
        return response;
    }
    let actor = match admin_session(&state, &headers).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let email = input
        .support_email
        .as_deref()
        .map(str::trim)
        .filter(|email| !email.is_empty());
    if email.is_some_and(|email| !valid_email(email)) {
        return error(
            StatusCode::BAD_REQUEST,
            "Enter a valid support email address",
        );
    }
    match state
        .institution
        .set_support_email(actor.user_id(), email)
        .await
    {
        Ok(()) => {
            Json(serde_json::json!({"schema_version":1,"support_email":email})).into_response()
        }
        Err(value) => institution_error(value),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn support_email_validation() {
        assert!(super::valid_email("help@institution.example"));
        for value in [
            "",
            "null",
            "a@b",
            "a@b..c",
            "a@b@c.test",
            "a@b.test?subject=bad",
            "a b@c.test",
        ] {
            assert!(!super::valid_email(value));
        }
    }
}
