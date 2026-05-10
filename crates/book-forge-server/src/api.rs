use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use serde::Serialize;
use uuid::Uuid;

use crate::{
    errors::ApiError,
    jobs::{AppState, CreateJobRequest, JobStatus, validate_create_request},
};

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/api/health", any(health_route))
        .route("/api/jobs", any(jobs_route))
        .route("/api/jobs/{id}", any(job_status_route))
        .route("/api/jobs/{id}/download", any(download_route))
        .fallback(api_not_found)
        .with_state(state)
}

async fn health_route(method: Method) -> Result<Json<HealthResponse>, ApiError> {
    if method != Method::GET {
        return Err(ApiError::method_not_allowed());
    }

    Ok(Json(HealthResponse { status: "healthy" }))
}

async fn jobs_route(
    State(state): State<AppState>,
    method: Method,
    body: Bytes,
) -> Result<Response, ApiError> {
    if method != Method::POST {
        return Err(ApiError::method_not_allowed());
    }

    let request = parse_create_request(&body)?;
    let summary = validate_create_request(request)?;
    let response = state.jobs.create_job(state.fetcher.clone(), summary).await;
    Ok((StatusCode::ACCEPTED, Json(response)).into_response())
}

async fn job_status_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    method: Method,
) -> Result<Response, ApiError> {
    if method != Method::GET {
        return Err(ApiError::method_not_allowed());
    }

    let id = parse_job_id(&id)?;
    let response =
        state.jobs.get_response(id).await.ok_or_else(|| {
            ApiError::not_found("job_not_found", "The requested job was not found.")
        })?;

    Ok(Json(response).into_response())
}

async fn download_route(
    State(state): State<AppState>,
    Path(id): Path<String>,
    method: Method,
) -> Result<Response, ApiError> {
    if method != Method::GET {
        return Err(ApiError::method_not_allowed());
    }

    let id = parse_job_id(&id)?;
    let Some((status, artifact)) = state.jobs.artifact(id).await else {
        return Err(ApiError::not_found(
            "job_not_found",
            "The requested job was not found.",
        ));
    };

    match (status, artifact) {
        (JobStatus::Completed, Some(artifact)) => {
            let disposition = format!(
                "attachment; filename=\"{}\"",
                safe_header_filename(&artifact.filename)
            );
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/epub+zip")
                .header(header::CONTENT_LENGTH, artifact.bytes.len().to_string())
                .header(header::CONTENT_DISPOSITION, disposition)
                .body(Body::from(artifact.bytes))
                .map_err(|_| {
                    ApiError::conflict(
                        "download_unavailable",
                        "The EPUB download could not be prepared.",
                    )
                })
        }
        (JobStatus::Completed, None) => Err(ApiError::conflict(
            "download_unavailable",
            "The completed EPUB artifact is not available.",
        )),
        (JobStatus::Failed, _) => Err(ApiError::conflict(
            "job_failed",
            "Failed jobs do not have downloadable EPUB artifacts.",
        )),
        (JobStatus::Queued | JobStatus::Running, _) => Err(ApiError::conflict(
            "job_not_completed",
            "The EPUB download is available only after the job completes.",
        )),
    }
}

async fn api_not_found() -> ApiError {
    ApiError::not_found(
        "api_route_not_found",
        "The requested API route was not found.",
    )
}

fn parse_create_request(body: &[u8]) -> Result<CreateJobRequest, ApiError> {
    if body.is_empty() {
        return Err(ApiError::validation(
            "Job creation requires a JSON request body.",
            vec!["body".to_string()],
        ));
    }

    serde_json::from_slice(body).map_err(|_| {
        ApiError::validation(
            "Job creation requires valid JSON fields.",
            vec!["body".to_string()],
        )
    })
}

fn parse_job_id(id: &str) -> Result<Uuid, ApiError> {
    Uuid::parse_str(id)
        .map_err(|_| ApiError::bad_request("invalid_job_id", "The job id is not valid."))
}

fn safe_header_filename(filename: &str) -> String {
    let mut safe = String::new();
    for character in filename.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, ' ' | '.' | '-' | '_') {
            safe.push(character);
        } else if !safe.ends_with('_') {
            safe.push('_');
        }
    }
    while safe.contains("..") {
        safe = safe.replace("..", ".");
    }
    let safe = safe.trim_matches([' ', '.']).to_string();
    let mut safe = if safe.is_empty() {
        "book-forge.epub".to_string()
    } else {
        safe
    };
    if !safe.to_ascii_lowercase().ends_with(".epub") {
        safe.push_str(".epub");
    }
    safe
}
