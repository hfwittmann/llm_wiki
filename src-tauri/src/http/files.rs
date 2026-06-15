//! HTTP handler for file preview bytes.

use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use serde::Deserialize;

use crate::http::auth::AuthUser;
use crate::http::error::ApiError;
use crate::http::AppState;
use crate::storage::paths::{resolve_under, resolve_project_path, PathError};

pub fn files_router() -> Router<AppState> {
    Router::new().route("/api/v1/files/raw", get(raw))
}

#[derive(Debug, Deserialize)]
struct RawQuery {
    /// Two accepted shapes, in priority order:
    ///   (a) `project_path` + `path` — project_path can be absolute (under
    ///       projects_root) or relative; `path` is project-relative.
    ///   (b) Only `path` — must be an absolute path under projects_root.
    /// Legacy callers in the migrated frontend send (b); newer callers should
    /// prefer (a) because it's path-safer.
    #[serde(default)]
    project_path: Option<String>,
    path: String,
}

async fn raw(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(q): Query<RawQuery>,
) -> Result<Response, ApiError> {
    let projects_root = &state.config.projects_root;

    let file_path = match q.project_path.as_deref() {
        Some(pp) if !pp.is_empty() => {
            let project_root = resolve_project_path(projects_root, pp).map_err(|e| {
                ApiError::bad_request("PATH_ESCAPE", e.to_string())
                    .with_details(serde_json::json!({ "requested": pp }))
            })?;
            resolve_under(&project_root, &q.path).map_err(|e| match e {
                PathError::NotFound => ApiError::new(
                    StatusCode::NOT_FOUND,
                    "NOT_FOUND",
                    format!("file not found: {}", q.path),
                ),
                _ => ApiError::bad_request("PATH_ESCAPE", e.to_string())
                    .with_details(serde_json::json!({ "requested": q.path })),
            })?
        }
        _ => {
            // Single absolute path — must be under projects_root.
            resolve_project_path(projects_root, &q.path).map_err(|e| match e {
                PathError::NotFound => ApiError::new(
                    StatusCode::NOT_FOUND,
                    "NOT_FOUND",
                    format!("file not found: {}", q.path),
                ),
                _ => ApiError::bad_request("PATH_ESCAPE", e.to_string())
                    .with_details(serde_json::json!({ "requested": q.path })),
            })?
        }
    };

    if !file_path.is_file() {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            format!("file not found: {}", q.path),
        ));
    }

    let bytes = tokio::fs::read(&file_path).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ApiError::new(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            format!("file not found: {}", q.path),
        ),
        _ => ApiError::internal(e.to_string()),
    })?;

    let mime = mime_guess::from_path(&file_path).first_or_octet_stream();
    let resp = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(Body::from(bytes))
        .unwrap();
    // No caching headers for now — the frontend can re-fetch as needed.
    Ok(resp)
}
