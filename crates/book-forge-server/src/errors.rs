use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ErrorPayload {
    pub error: ErrorBody,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ApiError {
    status: StatusCode,
    body: ErrorBody,
}

impl ApiError {
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ErrorBody {
                code: code.into(),
                message: sanitize_message(message.into()),
                fields: Vec::new(),
            },
        }
    }

    pub fn validation(message: impl Into<String>, fields: Vec<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            body: ErrorBody {
                code: "validation_failed".to_string(),
                message: sanitize_message(message.into()),
                fields,
            },
        }
    }

    pub fn bad_request(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn not_found(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, code, message)
    }

    pub fn conflict(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, code, message)
    }

    pub fn method_not_allowed() -> Self {
        Self::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
            "This HTTP method is not supported for the requested API route.",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorPayload { error: self.body })).into_response()
    }
}

pub fn sanitize_message(message: impl AsRef<str>) -> String {
    let mut sanitized = String::new();
    for character in message.as_ref().chars() {
        if character.is_control() {
            sanitized.push(' ');
        } else {
            sanitized.push(character);
        }
    }
    let sanitized = sanitized.split_whitespace().collect::<Vec<_>>().join(" ");
    if sanitized.is_empty() || contains_sensitive_marker(&sanitized) {
        "The request could not be completed safely.".to_string()
    } else {
        sanitized
    }
}

fn contains_sensitive_marker(message: &str) -> bool {
    let lower = message.to_lowercase();
    ["/home/", "\\users\\", "target/debug", "backtrace", "panic"]
        .iter()
        .any(|marker| lower.contains(marker))
}
