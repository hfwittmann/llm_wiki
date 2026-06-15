//! HTTP handlers for browsing the server-side projects-root directory tree.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use crate::http::auth::AuthUser;
use crate::http::error::ApiError;
use crate::http::AppState;
use crate::storage::paths::resolve_project_path;

pub fn fs_browser_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/fs/list", get(list))
        .route("/api/v1/fs/mkdir", post(mkdir))
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    #[serde(default)]
    path: String,
}

async fn list(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let target = if q.path.is_empty() {
        // List the root itself.
        state
            .config
            .projects_root
            .canonicalize()
            .map_err(|e| ApiError::internal(format!("projects_root not canonicalizable: {e}")))?
    } else {
        // `resolve_project_path` accepts both relative and absolute paths
        // (the latter must be under projects_root). The migrated frontend's
        // legacy `listDirectory(absolutePath)` callers send absolute paths.
        resolve_project_path(&state.config.projects_root, &q.path).map_err(|e| {
            ApiError::bad_request("PATH_ESCAPE", e.to_string())
                .with_details(serde_json::json!({ "requested": q.path }))
        })?
    };

    if !target.exists() {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            format!("path does not exist: {}", q.path),
        ));
    }
    if !target.is_dir() {
        return Err(ApiError::bad_request("NOT_A_DIRECTORY", "target is not a directory"));
    }

    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&target).map_err(|e| ApiError::internal(e.to_string()))? {
        let entry = entry.map_err(|e| ApiError::internal(e.to_string()))?;
        let path = entry.path();
        let is_dir = path.is_dir();
        let name = entry.file_name().to_string_lossy().to_string();
        let is_project = is_dir
            && (path.join(".llm-wiki/schema.md").exists() || path.join("schema.md").exists());
        let metadata = entry.metadata().ok();
        let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified_unix = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        entries.push(serde_json::json!({
            "name": name,
            "is_dir": is_dir,
            "is_project": is_project,
            "size": size,
            "modified_unix": modified_unix,
        }));
    }
    Ok(Json(serde_json::json!({ "entries": entries })))
}

#[derive(Debug, Deserialize)]
struct MkdirRequest {
    path: String,
}

async fn mkdir(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(req): Json<MkdirRequest>,
) -> Result<StatusCode, ApiError> {
    if req.path.is_empty() {
        return Err(ApiError::bad_request("BAD_REQUEST", "path must not be empty"));
    }
    // Path may not exist yet (we're about to create it), so we can't use resolve_under directly.
    // Reject .. and absolute paths manually, then join with projects_root.
    if std::path::Path::new(&req.path).is_absolute() {
        return Err(ApiError::bad_request("PATH_ESCAPE", "absolute paths not allowed"));
    }
    for component in std::path::Path::new(&req.path).components() {
        if matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::Prefix(_)
                | std::path::Component::RootDir
        ) {
            return Err(
                ApiError::bad_request("PATH_ESCAPE", ".. or absolute segments not allowed")
                    .with_details(serde_json::json!({ "requested": req.path })),
            );
        }
    }
    let target = state.config.projects_root.join(&req.path);
    std::fs::create_dir_all(&target).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(StatusCode::CREATED)
}
