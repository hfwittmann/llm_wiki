//! Tauri wrappers for `create_project` / `open_project`.
//!
//! `open_project_folder` is Tauri-shell-specific (uses `AppHandle` +
//! `tauri_plugin_opener`) and stays here. All pure logic lives in
//! `crate::core::project`.

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::panic_guard::run_guarded;


#[tauri::command]
pub fn create_project(name: String, path: String) -> Result<crate::types::wiki::WikiProject, String> {
    crate::core::project::create_project(name, path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_project(path: String) -> Result<crate::types::wiki::WikiProject, String> {
    crate::core::project::open_project(path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_project_folder(app: AppHandle, path: String) -> Result<(), String> {
    run_guarded("open_project_folder", || {
        let root = std::path::Path::new(&path);
        crate::core::project::validate_wiki_project_root(root)?;

        let canonical = root
            .canonicalize()
            .map_err(|e| format!("Failed to resolve project path '{}': {}", path, e))?;
        let canonical = canonical.to_string_lossy().to_string();

        match app.opener().open_path(canonical.clone(), None::<&str>) {
            Ok(()) => Ok(()),
            Err(open_err) => app
                .opener()
                .reveal_item_in_dir(canonical)
                .map_err(|reveal_err| {
                    format!(
                        "Failed to open project folder: {}; reveal fallback also failed: {}",
                        open_err, reveal_err
                    )
                }),
        }
    })
}
