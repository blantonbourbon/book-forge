use std::path::{Component, Path, PathBuf};

use axum::{
    body::Body,
    extract::State,
    http::{Method, StatusCode, Uri, header},
    response::Response,
};
use tokio::fs;

use crate::{errors::ApiError, jobs::AppState};

pub async fn serve_static(
    State(state): State<AppState>,
    method: Method,
    uri: Uri,
) -> Result<Response, ApiError> {
    if !matches!(method, Method::GET | Method::HEAD) {
        return Err(ApiError::method_not_allowed());
    }

    let Some(root) = state.static_root.as_deref() else {
        return Err(ApiError::not_found(
            "route_not_found",
            "The requested route was not found.",
        ));
    };

    let request_path = uri.path();
    let target = static_target(root, request_path)?;
    let bytes = fs::read(&target).await.map_err(|_| {
        ApiError::not_found(
            "static_asset_not_found",
            "The requested asset was not found.",
        )
    })?;

    let body = if method == Method::HEAD {
        Body::empty()
    } else {
        Body::from(bytes.clone())
    };

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type(&target))
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(body)
        .map_err(|_| {
            ApiError::not_found(
                "static_asset_not_found",
                "The requested asset was not found.",
            )
        })
}

fn static_target(root: &Path, request_path: &str) -> Result<PathBuf, ApiError> {
    reject_suspicious_path(request_path)?;

    let relative = request_path.trim_start_matches('/');
    let mut path = root.to_path_buf();

    if relative.is_empty() {
        path.push("index.html");
    } else {
        for segment in relative.split('/') {
            if segment.is_empty() || segment == "." || segment == ".." {
                return Err(traversal_error());
            }
            path.push(segment);
        }
    }

    let canonical = if path.is_file() {
        path.canonicalize().map_err(|_| {
            ApiError::not_found(
                "static_asset_not_found",
                "The requested asset was not found.",
            )
        })?
    } else if should_fallback_to_index(relative) {
        root.join("index.html").canonicalize().map_err(|_| {
            ApiError::not_found(
                "static_asset_not_found",
                "The requested asset was not found.",
            )
        })?
    } else {
        return Err(ApiError::not_found(
            "static_asset_not_found",
            "The requested asset was not found.",
        ));
    };

    ensure_within_root(root, &canonical)?;
    Ok(canonical)
}

fn reject_suspicious_path(request_path: &str) -> Result<(), ApiError> {
    let lower = request_path.to_ascii_lowercase();
    if lower.contains("..")
        || lower.contains("%2e")
        || lower.contains("%2f")
        || lower.contains("%5c")
        || lower.contains('\\')
    {
        return Err(traversal_error());
    }
    Ok(())
}

fn should_fallback_to_index(relative: &str) -> bool {
    relative.is_empty()
        || !relative
            .rsplit('/')
            .next()
            .is_some_and(|last| last.contains('.'))
}

fn ensure_within_root(root: &Path, path: &Path) -> Result<(), ApiError> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
        || !path.starts_with(root)
    {
        return Err(traversal_error());
    }
    Ok(())
}

fn traversal_error() -> ApiError {
    ApiError::bad_request(
        "static_path_rejected",
        "The requested static path was not accepted.",
    )
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
