//! Tauri command wrappers for the vectorstore (LanceDB).
//!
//! Each function here is a thin `#[tauri::command]` shim that delegates
//! immediately to `crate::core::vectorstore`. All logic, data types, and
//! tests live in that module.

// Re-export the data types that the Tauri serialisation layer needs to see
// at the command boundary (they are `pub` in core and referenced by return
// types, so Tauri's macro can reach them through `crate::core::vectorstore`
// — but making them visible here keeps the import paths unchanged for any
// code that was already importing from `commands::vectorstore`).
pub use crate::core::vectorstore::{ChunkSearchResult, ChunkUpsertInput, VectorSearchResult};

/// Upsert a page embedding into LanceDB (v1 legacy table).
#[tauri::command]
pub async fn vector_upsert(
    project_path: String,
    page_id: String,
    embedding: Vec<f32>,
) -> Result<(), String> {
    crate::core::vectorstore::vector_upsert(project_path, page_id, embedding).await
}

/// Search for similar pages by embedding vector (v1 legacy table).
#[tauri::command]
pub async fn vector_search(
    project_path: String,
    query_embedding: Vec<f32>,
    top_k: usize,
) -> Result<Vec<VectorSearchResult>, String> {
    crate::core::vectorstore::vector_search(project_path, query_embedding, top_k).await
}

/// Delete a page from the v1 vector index.
#[tauri::command]
pub async fn vector_delete(project_path: String, page_id: String) -> Result<(), String> {
    crate::core::vectorstore::vector_delete(project_path, page_id).await
}

/// Get count of indexed vectors (v1 legacy table).
#[tauri::command]
pub async fn vector_count(project_path: String) -> Result<usize, String> {
    crate::core::vectorstore::vector_count(project_path).await
}

/// Upsert a batch of chunks for a single page into the v2 chunk table.
#[tauri::command]
pub async fn vector_upsert_chunks(
    project_path: String,
    page_id: String,
    chunks: Vec<ChunkUpsertInput>,
) -> Result<(), String> {
    crate::core::vectorstore::vector_upsert_chunks(project_path, page_id, chunks).await
}

/// Top-K chunk search against the v2 chunk table.
#[tauri::command]
pub async fn vector_search_chunks(
    project_path: String,
    query_embedding: Vec<f32>,
    top_k: usize,
) -> Result<Vec<ChunkSearchResult>, String> {
    crate::core::vectorstore::vector_search_chunks(project_path, query_embedding, top_k).await
}

/// Delete every chunk belonging to a page from the v2 table.
#[tauri::command]
pub async fn vector_delete_page(project_path: String, page_id: String) -> Result<(), String> {
    crate::core::vectorstore::vector_delete_page(project_path, page_id).await
}

/// Total chunk count in the v2 table.
#[tauri::command]
pub async fn vector_count_chunks(project_path: String) -> Result<usize, String> {
    crate::core::vectorstore::vector_count_chunks(project_path).await
}

/// Drop the v2 chunk table entirely.
#[tauri::command]
pub async fn vector_clear_chunks(project_path: String) -> Result<(), String> {
    crate::core::vectorstore::vector_clear_chunks(project_path).await
}

/// Row count of the legacy v1 table (0 when absent).
#[tauri::command]
pub async fn vector_legacy_row_count(project_path: String) -> Result<usize, String> {
    crate::core::vectorstore::vector_legacy_row_count(project_path).await
}

/// Drop the legacy v1 table entirely.
#[tauri::command]
pub async fn vector_drop_legacy(project_path: String) -> Result<(), String> {
    crate::core::vectorstore::vector_drop_legacy(project_path).await
}
