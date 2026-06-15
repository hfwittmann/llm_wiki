//! HTTP handlers for browsing the server-side projects-root directory tree.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
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
        .route("/api/v1/fs/write", post(write))
        .route("/api/v1/fs/write_base64", post(write_base64))
        .route("/api/v1/fs/exists", get(exists))
        .route("/api/v1/fs/file", delete(delete_file))
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
    let target = resolve_writable_path(&state, &req.path)?;
    std::fs::create_dir_all(&target).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(StatusCode::CREATED)
}

// ── write text / base64 / exists / delete ────────────────────────────────────

#[derive(Debug, Deserialize)]
struct WriteRequest {
    path: String,
    content: String,
}

async fn write(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(req): Json<WriteRequest>,
) -> Result<StatusCode, ApiError> {
    let target = resolve_writable_path(&state, &req.path)?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ApiError::internal(e.to_string()))?;
    }
    std::fs::write(&target, req.content.as_bytes())
        .map_err(|e| ApiError::internal(format!("write failed for {}: {e}", req.path)))?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct WriteBase64Request {
    path: String,
    base64: String,
}

async fn write_base64(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(req): Json<WriteBase64Request>,
) -> Result<StatusCode, ApiError> {
    use base64::Engine;
    let target = resolve_writable_path(&state, &req.path)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(req.base64.as_bytes())
        .map_err(|e| ApiError::bad_request("BAD_BASE64", e.to_string()))?;
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ApiError::internal(e.to_string()))?;
    }
    std::fs::write(&target, &bytes)
        .map_err(|e| ApiError::internal(format!("write failed for {}: {e}", req.path)))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn exists(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    if q.path.is_empty() {
        return Err(ApiError::bad_request("BAD_REQUEST", "path must not be empty"));
    }
    let target = resolve_writable_path(&state, &q.path)?;
    Ok(Json(serde_json::json!({ "exists": target.exists() })))
}

#[derive(Debug, Deserialize)]
struct DeleteQuery {
    path: String,
}

async fn delete_file(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(q): Query<DeleteQuery>,
) -> Result<StatusCode, ApiError> {
    let target = resolve_writable_path(&state, &q.path)?;
    if !target.exists() {
        return Ok(StatusCode::NO_CONTENT);
    }
    if target.is_dir() {
        return Err(ApiError::bad_request(
            "NOT_A_FILE",
            "path is a directory; use a dedicated rmdir endpoint",
        ));
    }
    std::fs::remove_file(&target)
        .map_err(|e| ApiError::internal(format!("remove failed for {}: {e}", q.path)))?;
    Ok(StatusCode::NO_CONTENT)
}

/// Resolve a path that may not yet exist (e.g. for write/mkdir) under
/// projects_root, accepting both absolute paths already under the root and
/// relative paths. Rejects `..` segments and absolute paths outside the root.
fn resolve_writable_path(
    state: &AppState,
    raw: &str,
) -> Result<std::path::PathBuf, ApiError> {
    if raw.is_empty() {
        return Err(ApiError::bad_request("BAD_REQUEST", "path must not be empty"));
    }
    let p = std::path::Path::new(raw);
    let candidate = if p.is_absolute() {
        // Absolute path: must canonicalize to something under projects_root.
        // We canonicalize the ancestor that exists, since the path itself may
        // not exist yet.
        let mut ancestor = p;
        loop {
            if ancestor.exists() {
                break;
            }
            match ancestor.parent() {
                Some(parent) if parent != ancestor => ancestor = parent,
                _ => break,
            }
        }
        let canon_ancestor = ancestor
            .canonicalize()
            .map_err(|e| ApiError::bad_request("PATH_ESCAPE", e.to_string()))?;
        let canon_root = state
            .config
            .projects_root
            .canonicalize()
            .map_err(|e| ApiError::internal(format!("projects_root: {e}")))?;
        if !canon_ancestor.starts_with(&canon_root) {
            return Err(ApiError::bad_request(
                "PATH_ESCAPE",
                "absolute path is outside projects_root",
            ));
        }
        p.to_path_buf()
    } else {
        for component in p.components() {
            if matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::Prefix(_)
                    | std::path::Component::RootDir
            ) {
                return Err(ApiError::bad_request(
                    "PATH_ESCAPE",
                    ".. or root segments not allowed",
                ));
            }
        }
        state.config.projects_root.join(raw)
    };
    Ok(candidate)
}
