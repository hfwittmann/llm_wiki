//! HTTP handlers for source listing, ingest kickoff, and queue snapshot.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tower_cookies::Cookies;

use crate::core::events::EventSink;
use crate::http::auth::AuthUser;
use crate::http::error::ApiError;
use crate::http::session_event_sink::SessionEventSink;
use crate::http::AppState;
use crate::storage::paths::resolve_project_path;

pub fn sources_router() -> Router<AppState> {
    Router::new()
        .route("/api/v1/sources/ingest", post(ingest))
        .route("/api/v1/sources/list", get(list))
        .route("/api/v1/sources/ingest/queue", get(queue))
}

// ── ingest ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct IngestRequest {
    project_path: String,
    #[serde(default)]
    paths: Option<Vec<String>>,
}

async fn ingest(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    cookies: Cookies,
    Json(req): Json<IngestRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let project_root =
        resolve_project_path(&state.config.projects_root, &req.project_path).map_err(|e| {
            ApiError::bad_request("PATH_ESCAPE", e.to_string())
                .with_details(serde_json::json!({ "requested": req.project_path }))
        })?;

    let session_id = cookies
        .get(&state.config.session_cookie_name)
        .map(|c| c.value().to_string())
        .ok_or_else(ApiError::unauthenticated)?;

    let sink = Arc::new(SessionEventSink::new(state.session_bus.clone(), session_id))
        as Arc<dyn EventSink + Send + Sync + 'static>;

    let project_id = crate::core::project::project_id_from_canonical_path(&project_root);
    let project_root_str = project_root.to_string_lossy().to_string();

    tokio::task::spawn_blocking(move || {
        let result = if let Some(paths) = req.paths {
            // Selective re-scan: enqueue the provided paths then run a rescan
            // so that the queue-processor picks them up and emits events.
            let rels: BTreeSet<String> = paths.into_iter().collect();
            let root_pb = std::path::PathBuf::from(&project_root_str);
            match crate::core::ingest_queue::enqueue_paths(&root_pb, &project_id, rels) {
                Ok(()) => crate::core::file_sync::rescan_project_files(
                    &project_id,
                    &project_root_str,
                    None,
                    sink.as_ref(),
                ),
                Err(e) => Err(e),
            }
        } else {
            // Full project rescan.
            crate::core::file_sync::rescan_project_files(
                &project_id,
                &project_root_str,
                None,
                sink.as_ref(),
            )
        };
        if let Err(e) = result {
            eprintln!("[ingest] background rescan failed: {e}");
        }
    });

    Ok((StatusCode::ACCEPTED, Json(serde_json::json!({}))))
}

// ── list ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ListQuery {
    project_path: String,
}

async fn list(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let project_root =
        resolve_project_path(&state.config.projects_root, &q.project_path).map_err(|e| {
            ApiError::bad_request("PATH_ESCAPE", e.to_string())
                .with_details(serde_json::json!({ "requested": q.project_path }))
        })?;

    let raw_dir = project_root.join("raw");
    if !raw_dir.exists() {
        return Ok(Json(serde_json::json!({ "files": [] })));
    }

    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(&raw_dir)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let rel = path
            .strip_prefix(&raw_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = metadata.len();
        let modified_unix = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        files.push(serde_json::json!({
            "name": name,
            "rel_path": rel,
            "size": size,
            "modified_unix": modified_unix,
        }));
    }

    Ok(Json(serde_json::json!({ "files": files })))
}

// ── queue snapshot ────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct QueueQuery {
    project_path: String,
}

async fn queue(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(q): Query<QueueQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let project_root =
        resolve_project_path(&state.config.projects_root, &q.project_path).map_err(|e| {
            ApiError::bad_request("PATH_ESCAPE", e.to_string())
                .with_details(serde_json::json!({ "requested": q.project_path }))
        })?;

    let project_root_str = project_root.to_string_lossy().to_string();
    let file_queue = crate::core::file_sync::get_file_change_queue(&project_root_str)
        .map_err(|e| ApiError::internal(e))?;

    let value =
        serde_json::to_value(&file_queue).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(value))
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::http::Request;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::auth::sessions::Sessions;
    use crate::auth::users::{hash_password, Users};
    use crate::config::ServerConfig;
    use crate::http::main_router;
    use crate::storage::session_bus::SessionBus;
    use crate::storage::user_data::UserData;

    fn build_state_with_projects_root(
        username: &str,
        password: &str,
        projects_root: PathBuf,
        data_root: PathBuf,
    ) -> AppState {
        let hash = hash_password(password).unwrap();
        let users_path = data_root.join("users.toml");
        std::fs::write(
            &users_path,
            format!("[users.{username}]\npassword_hash = \"{hash}\"\n"),
        )
        .unwrap();
        let users = Users::load(&users_path).unwrap();
        let sessions = Sessions::open(&data_root.join("sessions")).unwrap();
        let user_data = UserData::new(data_root.clone());
        let bus = SessionBus::new();
        let cfg = ServerConfig {
            bind: "127.0.0.1".into(),
            port: 8080,
            projects_root,
            data_root,
            legacy_19828_enabled: true,
            session_cookie_name: "test_session".into(),
        };
        AppState {
            users: Arc::new(users),
            sessions,
            user_data,
            session_bus: bus,
            config: Arc::new(cfg),
            llm_client: Arc::new(crate::core::llm_client::LlmClient::new()),
        }
    }

    async fn do_login(app: axum::Router, username: &str, password: &str) -> String {
        let body = format!(r#"{{"username":"{username}","password":"{password}"}}"#);
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/login")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200, "login failed");
        resp.headers()
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_string()
    }

    // ── sources_ingest_without_cookie_is_401 ──────────────────────────────────

    #[tokio::test]
    async fn sources_ingest_without_cookie_is_401() {
        let dir = TempDir::new().unwrap();
        let projects_root = dir.path().join("projects");
        std::fs::create_dir_all(&projects_root).unwrap();
        let state = build_state_with_projects_root(
            "alice",
            "pw",
            projects_root,
            dir.path().to_path_buf(),
        );
        let app = main_router(state);
        let body = r#"{"project_path": "proj"}"#;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sources/ingest")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 401);
    }

    // ── sources_list_for_nonexistent_project_path_rejects ─────────────────────

    #[tokio::test]
    async fn sources_list_for_nonexistent_project_path_rejects() {
        let dir = TempDir::new().unwrap();
        let projects_root = dir.path().join("projects");
        std::fs::create_dir_all(&projects_root).unwrap();
        let state = build_state_with_projects_root(
            "alice",
            "pw",
            projects_root,
            dir.path().to_path_buf(),
        );
        let app = main_router(state.clone());
        let cookie = do_login(app.clone(), "alice", "pw").await;

        // Path traversal attempt.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sources/list?project_path=../etc")
                    .header("cookie", cookie)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 400);
        let body = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["error"]["code"], "PATH_ESCAPE");
    }

    // ── sources_list_for_empty_project_returns_empty_files ────────────────────

    #[tokio::test]
    async fn sources_list_for_empty_project_returns_empty_files() {
        let dir = TempDir::new().unwrap();
        let projects_root = dir.path().join("projects");
        // Create a project dir without a `raw/` subdirectory.
        std::fs::create_dir_all(projects_root.join("myproj")).unwrap();
        let state = build_state_with_projects_root(
            "alice",
            "pw",
            projects_root,
            dir.path().to_path_buf(),
        );
        let app = main_router(state.clone());
        let cookie = do_login(app.clone(), "alice", "pw").await;

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/sources/list?project_path=myproj")
                    .header("cookie", cookie)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(v["files"].as_array().unwrap().is_empty());
    }

    // ── sources_ingest_returns_202_and_schedules_work ─────────────────────────

    #[tokio::test]
    async fn sources_ingest_returns_202_and_schedules_work() {
        let dir = TempDir::new().unwrap();
        let projects_root = dir.path().join("projects");
        let proj_dir = projects_root.join("myproj");
        // Provide minimum scaffolding so rescan_project_files can initialise
        // the sync directory without error.
        std::fs::create_dir_all(proj_dir.join("raw")).unwrap();
        let state = build_state_with_projects_root(
            "alice",
            "pw",
            projects_root,
            dir.path().to_path_buf(),
        );
        let app = main_router(state.clone());
        let cookie = do_login(app.clone(), "alice", "pw").await;

        let body = r#"{"project_path": "myproj"}"#;
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sources/ingest")
                    .header("content-type", "application/json")
                    .header("cookie", cookie)
                    .body(axum::body::Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), 202);
        let body = to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Response is an empty object.
        assert!(v.as_object().unwrap().is_empty());
    }
}
