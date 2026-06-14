//! Pure project management logic — no Tauri, no AppHandle.
//!
//! `create_project` and `open_project` are called by thin `#[tauri::command]`
//! wrappers in `commands::project`. Adding HTTP handlers in Phase 4 should
//! only require calling these functions directly.

use std::fs;
use std::path::Path;

use chrono::Local;

use crate::types::wiki::WikiProject;

// ──────────────────────────────────────────────────────────────────────────
// Error type
// ──────────────────────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
    #[error("project already exists: {0}")]
    AlreadyExists(String),
    #[error("project not found: {0}")]
    NotFound(String),
    #[error("template error: {0}")]
    Template(String),
    #[error("internal: {0}")]
    Internal(String),
}

// ProjectError stores String-based errors from the legacy helpers, so
// implement From<String> for easy conversion.
impl From<String> for ProjectError {
    fn from(s: String) -> Self {
        ProjectError::Internal(s)
    }
}

// ──────────────────────────────────────────────────────────────────────────
// Public helpers (used by commands/project.rs for open_project_folder)
// ──────────────────────────────────────────────────────────────────────────

/// Validate that a path is a directory containing a valid wiki project
/// structure (`schema.md` + `wiki/` subdirectory).
pub fn validate_wiki_project_root(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Err(format!("Path does not exist: '{}'", root.display()));
    }
    if !root.is_dir() {
        return Err(format!("Path is not a directory: '{}'", root.display()));
    }

    if !root.join("schema.md").exists() {
        return Err(format!(
            "Not a valid wiki project (missing schema.md): '{}'",
            root.display()
        ));
    }
    if !root.join("wiki").is_dir() {
        return Err(format!(
            "Not a valid wiki project (missing wiki/ directory): '{}'",
            root.display()
        ));
    }

    Ok(())
}

// ──────────────────────────────────────────────────────────────────────────
// Internal helpers
// ──────────────────────────────────────────────────────────────────────────

fn write_file_inner(path: std::path::PathBuf, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            format!(
                "Failed to create parent dirs for '{}': {}",
                path.display(),
                e
            )
        })?;
    }
    fs::write(&path, contents)
        .map_err(|e| format!("Failed to write file '{}': {}", path.display(), e))
}

fn create_project_impl(name: String, path: String) -> Result<WikiProject, String> {
    let root = Path::new(&path).join(&name);

    if root.exists() {
        return Err(format!("Directory already exists: '{}'", root.display()));
    }

    // Create all required subdirectories
    let dirs = [
        "raw/sources",
        "raw/assets",
        "wiki/entities",
        "wiki/concepts",
        "wiki/sources",
        "wiki/queries",
        "wiki/comparisons",
        "wiki/synthesis",
    ];
    for dir in &dirs {
        fs::create_dir_all(root.join(dir))
            .map_err(|e| format!("Failed to create directory '{}': {}", dir, e))?;
    }

    let today = Local::now().format("%Y-%m-%d").to_string();

    // schema.md
    let schema_content = format!(
        r#"# Wiki Schema

## Page Types

| Type | Directory | Purpose |
|------|-----------|---------|
| entity | wiki/entities/ | Named things (models, companies, people, datasets) |
| concept | wiki/concepts/ | Ideas, techniques, phenomena |
| source | wiki/sources/ | Papers, articles, talks, blog posts |
| query | wiki/queries/ | Open questions under investigation |
| comparison | wiki/comparisons/ | Side-by-side analysis of related entities |
| synthesis | wiki/synthesis/ | Cross-cutting summaries and conclusions |

## Naming Conventions

- Files: `kebab-case.md`
- Entities: match official name where possible (e.g., `gpt-4.md`, `openai.md`)
- Concepts: descriptive noun phrases (e.g., `chain-of-thought.md`)
- Sources: `author-year-slug.md` (e.g., `wei-2022-chain-of-thought.md`)
- Queries: question as slug (e.g., `does-scale-improve-reasoning.md`)

## Frontmatter

All pages must include YAML frontmatter:

```yaml
---
type: entity | concept | source | query | comparison | synthesis | overview
title: Human-readable title
tags: []
related: []
created: YYYY-MM-DD
updated: YYYY-MM-DD
---
```

Source pages also include:
```yaml
authors: []
year: YYYY
url: ""
venue: ""
```

## Index Format

`wiki/index.md` lists all pages grouped by type. Each entry:
```
- [[page-slug]] — one-line description
```

## Log Format

`wiki/log.md` records research activity in reverse chronological order:
```
## YYYY-MM-DD

- Action taken / finding noted
```

## Cross-referencing Rules

- Use `[[page-slug]]` syntax to link between wiki pages
- Every entity and concept should appear in `wiki/index.md`
- Queries link to the sources and concepts they draw on
- Synthesis pages cite all contributing sources via `related:`

## Contradiction Handling

When sources contradict each other:
1. Note the contradiction in the relevant concept or entity page
2. Create or update a query page to track the open question
3. Link both sources from the query page
4. Resolve in a synthesis page once sufficient evidence exists
"#
    );
    write_file_inner(root.join("schema.md"), &schema_content)?;

    // purpose.md
    let purpose_content = r#"# Project Purpose

## Goal

<!-- What are you trying to understand or build? -->

## Key Questions

<!-- List the primary questions driving this research -->

1.
2.
3.

## Scope

<!-- What is in scope? What is explicitly out of scope? -->

**In scope:**
-

**Out of scope:**
-

## Thesis

<!-- Your current working hypothesis or conclusion (update as research progresses) -->

> TBD
"#;
    write_file_inner(root.join("purpose.md"), purpose_content)?;

    // wiki/index.md
    let index_content = r#"# Wiki Index

## Entities

## Concepts

## Sources

## Queries

## Comparisons

## Synthesis
"#;
    write_file_inner(root.join("wiki/index.md"), index_content)?;

    // wiki/log.md
    let log_content = format!(
        r#"# Research Log

## {today}

- Project created
"#
    );
    write_file_inner(root.join("wiki/log.md"), &log_content)?;

    // wiki/overview.md
    let overview_content = r#"---
type: overview
title: Project Overview
tags: []
related: []
---

# Overview

<!-- Provide a high-level summary of what this wiki covers and its current state. Update regularly as understanding deepens. -->
"#;
    write_file_inner(root.join("wiki/overview.md"), overview_content)?;

    // .obsidian config for Obsidian compatibility
    fs::create_dir_all(root.join(".obsidian"))
        .map_err(|e| format!("Failed to create .obsidian: {}", e))?;

    // Obsidian app config: set attachment folder, exclude hidden dirs
    let obsidian_app_config = r#"{
  "attachmentFolderPath": "raw/assets",
  "userIgnoreFilters": [
    ".cache",
    ".llm-wiki",
    ".superpowers"
  ],
  "useMarkdownLinks": false,
  "newLinkFormat": "shortest",
  "showUnsupportedFiles": false
}"#;
    write_file_inner(root.join(".obsidian/app.json"), obsidian_app_config)?;

    // Obsidian appearance: dark mode
    let obsidian_appearance = r#"{
  "baseFontSize": 16,
  "theme": "obsidian"
}"#;
    write_file_inner(root.join(".obsidian/appearance.json"), obsidian_appearance)?;

    // Enable graph view and backlinks core plugins
    let obsidian_core_plugins = r#"{
  "file-explorer": true,
  "global-search": true,
  "graph": true,
  "backlink": true,
  "tag-pane": true,
  "page-preview": true,
  "outgoing-link": true,
  "starred": true
}"#;
    write_file_inner(
        root.join(".obsidian/core-plugins.json"),
        obsidian_core_plugins,
    )?;

    Ok(WikiProject {
        name,
        // Forward slashes for cross-platform consistency in the TS layer.
        path: root.to_string_lossy().replace('\\', "/"),
    })
}

// ──────────────────────────────────────────────────────────────────────────
// Public API
// ──────────────────────────────────────────────────────────────────────────

/// Create a new wiki project at `<path>/<name>` with the standard directory
/// layout and seed files.
pub fn create_project(name: String, path: String) -> Result<WikiProject, ProjectError> {
    use crate::panic_guard::run_guarded;
    run_guarded("create_project", || create_project_impl(name, path))
        .map_err(ProjectError::Internal)
}

/// Open an existing wiki project by validating the directory structure and
/// returning its metadata.
pub fn open_project(path: String) -> Result<WikiProject, ProjectError> {
    use crate::panic_guard::run_guarded;
    run_guarded("open_project", || {
        let root = Path::new(&path);

        validate_wiki_project_root(root)?;

        // Derive project name from the directory name
        let name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
            .to_string();

        Ok(WikiProject {
            name,
            // Forward slashes for cross-platform consistency in the TS layer.
            path: path.replace('\\', "/"),
        })
    })
    .map_err(ProjectError::Internal)
}

// ──────────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let p = std::env::temp_dir().join(format!("llm-wiki-projtest-{}-{}", ts, id));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn create_project_produces_expected_structure() {
        let base = tmp_dir();
        let base_str = base.to_string_lossy().to_string();

        let result = create_project("my-wiki".to_string(), base_str.clone());
        let project = result.expect("create_project should succeed");

        assert_eq!(project.name, "my-wiki");
        let root = base.join("my-wiki");
        assert!(root.join("schema.md").exists(), "schema.md missing");
        assert!(root.join("purpose.md").exists(), "purpose.md missing");
        assert!(root.join("wiki/index.md").exists(), "wiki/index.md missing");
        assert!(root.join("wiki/log.md").exists(), "wiki/log.md missing");
        assert!(
            root.join("wiki/overview.md").exists(),
            "wiki/overview.md missing"
        );
        assert!(root.join(".obsidian").is_dir(), ".obsidian missing");
        assert!(
            root.join(".obsidian/app.json").exists(),
            ".obsidian/app.json missing"
        );
        assert!(
            root.join("raw/sources").is_dir(),
            "raw/sources dir missing"
        );
        assert!(root.join("wiki/entities").is_dir(), "wiki/entities missing");
    }

    #[test]
    fn create_project_path_uses_forward_slashes() {
        let base = tmp_dir();
        let project = create_project("slash-test".to_string(), base.to_string_lossy().to_string())
            .expect("create_project should succeed");
        assert!(
            !project.path.contains('\\'),
            "path should not contain backslashes: {}",
            project.path
        );
    }

    #[test]
    fn create_project_fails_if_directory_already_exists() {
        let base = tmp_dir();
        let name = "already-exists";
        std::fs::create_dir_all(base.join(name)).unwrap();

        let result = create_project(name.to_string(), base.to_string_lossy().to_string());
        assert!(result.is_err(), "expected Err when directory already exists");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("already exists") || msg.contains("Directory already exists"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn open_project_returns_correct_name_and_path() {
        let base = tmp_dir();
        let base_str = base.to_string_lossy().to_string();

        // Create first so the structure is valid.
        create_project("open-me".to_string(), base_str.clone()).unwrap();

        let project_path = base.join("open-me").to_string_lossy().to_string();
        let result = open_project(project_path.clone());
        let project = result.expect("open_project should succeed");

        assert_eq!(project.name, "open-me");
        assert!(!project.path.contains('\\'), "path should use forward slashes");
    }

    #[test]
    fn open_project_fails_on_missing_path() {
        let result = open_project("/tmp/llm-wiki-this-path-does-not-exist-99999".to_string());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("does not exist") || msg.contains("Path does not exist"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn open_project_fails_when_schema_md_missing() {
        let base = tmp_dir();
        // Create wiki/ subdir but NOT schema.md.
        std::fs::create_dir_all(base.join("wiki")).unwrap();

        let result = open_project(base.to_string_lossy().to_string());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("schema.md"),
            "expected mention of schema.md in: {msg}"
        );
    }

    #[test]
    fn open_project_fails_when_wiki_dir_missing() {
        let base = tmp_dir();
        // Write schema.md but omit the wiki/ dir.
        std::fs::write(base.join("schema.md"), "# Schema").unwrap();

        let result = open_project(base.to_string_lossy().to_string());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("wiki"),
            "expected mention of wiki/ dir in: {msg}"
        );
    }

    #[test]
    fn validate_wiki_project_root_accepts_valid_project() {
        let base = tmp_dir();
        let base_str = base.to_string_lossy().to_string();
        create_project("valid".to_string(), base_str).unwrap();

        let root = base.join("valid");
        assert!(validate_wiki_project_root(&root).is_ok());
    }
}
