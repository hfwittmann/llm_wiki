# Browser/LAN GUI for LLM Wiki — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the LLM Wiki desktop app (Tauri shell + React frontend + Rust backend) to a single Rust HTTP-server binary that serves the React frontend over the LAN. Multi-user (per-user accounts, chat, LLM config), shared project state (wiki, sources, vector store). The Tauri shell is removed; one binary runs on a VM (or laptop) and is reached by any browser on the LAN.

**Architecture:** Big-bang port on a feature branch (`main` keeps the working desktop app). One Rust binary embedding the React frontend via `rust-embed`. Two TCP listeners — `0.0.0.0:<port>` with auth, `127.0.0.1:19828` without (back-compat for the bundled MCP server). `axum` for HTTP, `sled` for persisted sessions, `argon2` for password hashing. Frontend rewires from Tauri IPC (`invoke()`/`listen()`) to HTTP (`fetch()`) + SSE (`EventSource`).

**Tech stack:** Rust (axum, tokio, sled, argon2, toml, rust-embed, reqwest, lancedb, pdfium-render — existing), React 19 + Vite 8 + TypeScript + Tailwind (existing, transport rewired).

**Source spec:** `plans/2026-06-14-browser-lan-gui-design.md`

---

## How this plan is scoped

This port is ~6 weeks of work. A single document with every TDD step for every module would be 3000+ lines and stale before phase 3 lands. Instead:

- **Phase 1 (this document, in full TDD detail)** — backend foundation modules that have no upstream dependencies and are pure logic with tests: `paths`, `users`, `sessions`, `user_data`. These ship correctly or the entire auth/path-safety story is broken; they're the right place to be rigorous up front.
- **Phases 2–7 (outlined here at file/test granularity)** — locked-in file paths, key signatures, key tests, but not step-by-step. After Phase 1 lands, write a detailed plan for Phase 2 (and so on) that incorporates what was learned.

Each phase produces a coherent slice of the codebase even though the *app* isn't end-to-end runnable until Phase 6.

---

## Phase overview

| Phase | What lands | Verifiable by |
|---|---|---|
| **1. Backend foundation** | `storage/paths.rs`, `auth/users.rs`, `auth/sessions.rs`, `storage/user_data.rs` (+ tests). New Cargo deps added. No HTTP wired yet. | `cargo test` (new tests green) |
| **2. HTTP server skeleton** | `axum` server, dual listeners, `/auth/*` routes, session middleware, SSE skeleton, `rust-embed` for SPA serving. `main.rs` opens a listening server. | `curl localhost:<port>/api/v1/auth/whoami` returns 401 |
| **3. Core extraction** | Business logic moves out of `#[tauri::command]` wrappers into `core/`. `AppHandle` references removed; replaced with explicit args + `EventSink` trait. Desktop app still runs (commands are thin wrappers over `core/`). | `cargo test`; desktop app still launches and ingests a doc |
| **4. HTTP handlers** | `/projects/*`, `/wiki/*`, `/sources/*`, `/chat/*`, `/config/*`, `/fs/*`, `/files/*`, `/proxy/llm`, `/agent/*` — thin axum handlers over `core/`. Per-handler tests. | `cargo test`; `curl` exercises each endpoint |
| **5. Frontend transport rewire** | `src/lib/api.ts` + `src/lib/events.ts` created. Every `@tauri-apps/*` call site rewritten to use them. Settings stops using `@tauri-apps/plugin-store` → uses `/api/v1/config`. | `vitest`; frontend builds without `@tauri-apps/*` imports |
| **6. New UI screens** | `<LoginView>`, `<FolderBrowserDialog>`, `<UserBadge>`, top-bar project switcher; settings reorganized into Personal/Project sections. | Manually log in, browse folders, switch projects in a browser |
| **7. Cleanup** | `src-tauri/` renamed to `src-server/`; Tauri-specific files deleted; `tauri = ...` and `@tauri-apps/*` deps removed; `vite.config.ts` + `package.json` scripts updated; `manual-test-plan.md` checklist run. | `cargo build` produces only `llm-wiki-server`; `npm run build` produces `dist/` with no Tauri-specific code; manual smoke test on Mac + one other browser; two-user LAN smoke if possible. |

---

# Phase 1 — Backend foundation

**Phase goal:** Pure-Rust modules for the four chokepoint concerns (path safety, user accounts, sessions, per-user storage). All have inline `#[cfg(test)] mod tests` with high coverage. No `axum`, no `tauri`, no HTTP — they import only `std`, `serde`, `argon2`, `sled`, `toml`, `rand`, `chrono`. Wired up in later phases.

**Branch:** `feat/browser-lan-port`. Created from `main`. All Phase 1 commits land on this branch.

**Where the modules live:** `src-tauri/src/` for now (the rename to `src-server/` happens in Phase 7). All new modules go under `src-tauri/src/storage/` and `src-tauri/src/auth/`. The existing `src-tauri/src/api_server.rs`, `commands/`, `clip_server.rs`, etc. are not touched in this phase.

---

## Task 1.1 — Add Cargo dependencies

**Files:**
- Modify: `src-tauri/Cargo.toml`

**Background:** Phase 1 needs `argon2` (password hashing), `sled` (embedded KV store for sessions), `toml` (parse `users.toml`), `rand` (generate session IDs), `parking_lot` (faster Mutex for in-memory hot paths than `std::sync::Mutex`). All others are existing or for later phases.

- [ ] **Step 1: Add deps to `Cargo.toml`**

Open `src-tauri/Cargo.toml` and add the following lines under `[dependencies]` (after the existing entries, before `[dev-dependencies]`):

```toml
# Phase 1 (backend foundation): password hashing, persisted sessions,
# user-account TOML, session-id randomness, faster mutex for hot paths.
argon2 = "0.5"
password-hash = { version = "0.5", features = ["alloc"] }
sled = "0.34"
toml = "0.8"
rand = "0.8"
parking_lot = "0.12"
```

- [ ] **Step 2: Verify the workspace builds**

Run: `(cd src-tauri && cargo check)`
Expected: builds successfully; only warnings about new unused deps are acceptable.

- [ ] **Step 3: Commit**

```bash
git checkout -b feat/browser-lan-port  # if not already on the branch
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "build: add argon2, sled, toml, rand, parking_lot for browser/LAN port phase 1"
```

---

## Task 1.2 — `storage/paths.rs` — path-traversal chokepoint

**Files:**
- Create: `src-tauri/src/storage/mod.rs`
- Create: `src-tauri/src/storage/paths.rs`
- Modify: `src-tauri/src/lib.rs` (declare `mod storage;`)

**Background:** The single function `resolve_under(root, requested)` is the chokepoint every filesystem-touching handler will go through in later phases. A bug here is a remote-file-read vulnerability. We TDD it with explicit edge cases plus a coverage check that requires every error variant.

**API to land:**

```rust
pub fn resolve_under(root: &Path, requested: &str) -> Result<PathBuf, PathError>;

pub enum PathError {
    Absolute,        // requested started with `/` or a Windows drive
    Traversal,       // any segment was `..`
    Empty,           // requested was empty after trimming
    NotFound,        // canonicalize failed (path doesn't exist)
    Escape,          // canonical path is outside root
    Invalid(String), // anything else (non-UTF-8, etc.)
}
```

The function rejects pre-canonicalize on `Absolute`/`Traversal`/`Empty`, then canonicalizes, then verifies the canonical path is still under root (catches symlinks-out).

- [ ] **Step 1: Create the module skeleton**

Create `src-tauri/src/storage/mod.rs`:

```rust
pub mod paths;
```

Create `src-tauri/src/storage/paths.rs`:

```rust
use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PathError {
    #[error("path is absolute")]
    Absolute,
    #[error("path contains `..` segment")]
    Traversal,
    #[error("path is empty")]
    Empty,
    #[error("path not found")]
    NotFound,
    #[error("canonical path escapes root")]
    Escape,
    #[error("invalid path: {0}")]
    Invalid(String),
}

pub fn resolve_under(root: &Path, requested: &str) -> Result<PathBuf, PathError> {
    todo!()
}
```

Note: this uses `thiserror`. Check `Cargo.toml`; if `thiserror` isn't already a dep, add `thiserror = "1"` under `[dependencies]` and rerun `cargo check`. The Rust ecosystem broadly uses `thiserror` for error enums; if the project has a preference (e.g. hand-rolled `Display`), follow that instead.

- [ ] **Step 2: Modify `src-tauri/src/lib.rs` to declare the module**

Add `pub mod storage;` near the top of `src-tauri/src/lib.rs` (alongside other module declarations).

If `lib.rs` doesn't currently declare modules at the top level (it might be a `pub use commands::*` re-export pattern), add the line in whichever location keeps Rust's module system happy. Run `cargo check` to confirm.

- [ ] **Step 3: Write the failing happy-path test**

Append to `src-tauri/src/storage/paths.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup() -> TempDir {
        let dir = TempDir::new().expect("create tempdir");
        fs::create_dir_all(dir.path().join("sub/nested")).unwrap();
        fs::write(dir.path().join("sub/nested/file.txt"), b"hi").unwrap();
        dir
    }

    #[test]
    fn resolves_a_valid_relative_path() {
        let dir = setup();
        let resolved = resolve_under(dir.path(), "sub/nested/file.txt").unwrap();
        assert_eq!(resolved, dir.path().canonicalize().unwrap().join("sub/nested/file.txt"));
    }
}
```

`tempfile` is needed. If it's not in `[dev-dependencies]` already, add `tempfile = "3"` under `[dev-dependencies]`.

- [ ] **Step 4: Run the test, expect it to fail**

Run: `(cd src-tauri && cargo test --lib storage::paths -- --nocapture)`
Expected: test runs but panics inside `todo!()`.

- [ ] **Step 5: Implement the function**

Replace the `todo!()` body in `src-tauri/src/storage/paths.rs` with:

```rust
pub fn resolve_under(root: &Path, requested: &str) -> Result<PathBuf, PathError> {
    let trimmed = requested.trim();
    if trimmed.is_empty() {
        return Err(PathError::Empty);
    }

    let req_path = Path::new(trimmed);

    // Reject pre-canonicalize: absolute paths, drive letters, traversal segments.
    if req_path.is_absolute() {
        return Err(PathError::Absolute);
    }
    for component in req_path.components() {
        match component {
            Component::ParentDir => return Err(PathError::Traversal),
            Component::Prefix(_) | Component::RootDir => return Err(PathError::Absolute),
            Component::CurDir | Component::Normal(_) => {}
        }
    }

    // Canonicalize root once for the prefix comparison. We accept the
    // overhead per-call for v1; later we can cache the canonical root.
    let root_canon = root
        .canonicalize()
        .map_err(|e| PathError::Invalid(format!("root not canonicalizable: {e}")))?;

    let joined = root_canon.join(req_path);
    let canon = joined.canonicalize().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => PathError::NotFound,
        _ => PathError::Invalid(e.to_string()),
    })?;

    if !canon.starts_with(&root_canon) {
        return Err(PathError::Escape);
    }
    Ok(canon)
}
```

- [ ] **Step 6: Run the happy-path test, expect it to pass**

Run: `(cd src-tauri && cargo test --lib storage::paths::tests::resolves_a_valid_relative_path)`
Expected: pass.

- [ ] **Step 7: Add the edge-case tests**

Append to the `#[cfg(test)] mod tests` block in `src-tauri/src/storage/paths.rs`:

```rust
    #[test]
    fn rejects_empty_path() {
        let dir = setup();
        assert!(matches!(resolve_under(dir.path(), ""), Err(PathError::Empty)));
        assert!(matches!(resolve_under(dir.path(), "   "), Err(PathError::Empty)));
    }

    #[test]
    fn rejects_absolute_unix_path() {
        let dir = setup();
        assert!(matches!(resolve_under(dir.path(), "/etc/passwd"), Err(PathError::Absolute)));
    }

    #[test]
    fn rejects_parent_traversal_segment() {
        let dir = setup();
        assert!(matches!(resolve_under(dir.path(), ".."), Err(PathError::Traversal)));
        assert!(matches!(resolve_under(dir.path(), "sub/../.."), Err(PathError::Traversal)));
        assert!(matches!(resolve_under(dir.path(), "sub/../../etc"), Err(PathError::Traversal)));
    }

    #[test]
    fn accepts_curdir_segments() {
        let dir = setup();
        let resolved = resolve_under(dir.path(), "./sub/./nested/file.txt").unwrap();
        assert!(resolved.ends_with("file.txt"));
    }

    #[test]
    fn returns_not_found_for_missing() {
        let dir = setup();
        assert!(matches!(
            resolve_under(dir.path(), "does/not/exist"),
            Err(PathError::NotFound)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_pointing_outside_root() {
        use std::os::unix::fs::symlink;
        let dir = setup();
        let outside = TempDir::new().unwrap();
        fs::write(outside.path().join("secret.txt"), b"hush").unwrap();
        symlink(outside.path().join("secret.txt"), dir.path().join("link_out")).unwrap();
        assert!(matches!(
            resolve_under(dir.path(), "link_out"),
            Err(PathError::Escape)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn accepts_symlink_pointing_inside_root() {
        use std::os::unix::fs::symlink;
        let dir = setup();
        symlink(
            dir.path().join("sub/nested/file.txt"),
            dir.path().join("link_in"),
        )
        .unwrap();
        let resolved = resolve_under(dir.path(), "link_in").unwrap();
        assert!(resolved.ends_with("file.txt"));
    }

    #[test]
    fn idempotent_on_canonical_paths() {
        let dir = setup();
        let first = resolve_under(dir.path(), "sub/nested/file.txt").unwrap();
        let rel = first
            .strip_prefix(dir.path().canonicalize().unwrap())
            .unwrap()
            .to_string_lossy()
            .to_string();
        let second = resolve_under(dir.path(), &rel).unwrap();
        assert_eq!(first, second);
    }
```

- [ ] **Step 8: Run all `paths` tests, expect them to pass**

Run: `(cd src-tauri && cargo test --lib storage::paths)`
Expected: all 8 tests pass.

If any fail, fix the implementation; do not relax the test.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/storage/ src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(storage): add resolve_under path-traversal chokepoint with tests"
```

---

## Task 1.3 — `auth/users.rs` — accounts loaded from `users.toml`

**Files:**
- Create: `src-tauri/src/auth/mod.rs`
- Create: `src-tauri/src/auth/users.rs`
- Modify: `src-tauri/src/lib.rs` (declare `mod auth;`)

**Background:** The admin maintains a TOML file mapping usernames to argon2 password hashes. The module loads the file, exposes `verify_password(username, plaintext) -> Result<User>`, and provides `hash_password(plaintext) -> String` for the user-management CLI we'll write in Phase 7. There is no signup flow.

**`users.toml` format:**

```toml
[users.alice]
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$..."

[users.bob]
password_hash = "$argon2id$v=19$m=19456,t=2,p=1$..."
```

**API to land:**

```rust
pub struct User {
    pub id: String,       // username, lowercased
    pub username: String, // username as stored (original case)
}

pub struct Users {
    by_id: HashMap<String, UserRecord>,
}

impl Users {
    pub fn load(path: &Path) -> Result<Self, UsersError>;
    pub fn verify_password(&self, username: &str, plaintext: &str) -> Result<User, AuthError>;
}

pub fn hash_password(plaintext: &str) -> Result<String, AuthError>;
```

- [ ] **Step 1: Create the module skeleton**

Create `src-tauri/src/auth/mod.rs`:

```rust
pub mod users;
```

Create `src-tauri/src/auth/users.rs`:

```rust
use std::collections::HashMap;
use std::path::Path;

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use serde::Deserialize;

#[derive(Debug, thiserror::Error)]
pub enum UsersError {
    #[error("users.toml not found: {0}")]
    NotFound(String),
    #[error("users.toml could not be read: {0}")]
    Io(#[from] std::io::Error),
    #[error("users.toml is malformed: {0}")]
    Malformed(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,
    #[error("password hashing failed: {0}")]
    Hash(String),
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub username: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UserRecord {
    password_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UsersFile {
    #[serde(default)]
    users: HashMap<String, UserRecord>,
}

#[derive(Debug, Clone, Default)]
pub struct Users {
    by_id: HashMap<String, UserRecord>,
    display_names: HashMap<String, String>,
}

impl Users {
    pub fn load(path: &Path) -> Result<Self, UsersError> {
        todo!()
    }

    pub fn verify_password(&self, username: &str, plaintext: &str) -> Result<User, AuthError> {
        todo!()
    }
}

pub fn hash_password(plaintext: &str) -> Result<String, AuthError> {
    todo!()
}
```

- [ ] **Step 2: Add `auth` module declaration in `lib.rs`**

Add `pub mod auth;` to `src-tauri/src/lib.rs` alongside the other module declarations.

- [ ] **Step 3: Run `cargo check` to confirm the skeleton compiles**

Run: `(cd src-tauri && cargo check)`
Expected: builds (with warnings about unused). If `serde::Deserialize` on `UsersFile` fails to derive because `serde`'s `derive` feature isn't enabled, confirm `Cargo.toml` has `serde = { version = "1", features = ["derive"] }` (it does in the current file).

- [ ] **Step 4: Write the failing tests**

Append to `src-tauri/src/auth/users.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write_users_toml(dir: &TempDir, contents: &str) -> std::path::PathBuf {
        let path = dir.path().join("users.toml");
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn hash_then_verify_roundtrip() {
        let hash = hash_password("correct horse battery staple").unwrap();
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(
            &dir,
            &format!(
                "[users.alice]\npassword_hash = \"{}\"\n",
                hash.replace('\\', "\\\\")
            ),
        );
        let users = Users::load(&path).unwrap();
        let user = users
            .verify_password("alice", "correct horse battery staple")
            .unwrap();
        assert_eq!(user.id, "alice");
    }

    #[test]
    fn rejects_wrong_password() {
        let hash = hash_password("right").unwrap();
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(
            &dir,
            &format!("[users.alice]\npassword_hash = \"{}\"\n", hash),
        );
        let users = Users::load(&path).unwrap();
        let result = users.verify_password("alice", "wrong");
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[test]
    fn rejects_unknown_user_with_same_error_as_wrong_password() {
        let hash = hash_password("right").unwrap();
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(
            &dir,
            &format!("[users.alice]\npassword_hash = \"{}\"\n", hash),
        );
        let users = Users::load(&path).unwrap();
        let result = users.verify_password("nobody", "anything");
        assert!(matches!(result, Err(AuthError::InvalidCredentials)));
    }

    #[test]
    fn username_is_case_insensitive_for_lookup() {
        let hash = hash_password("pw").unwrap();
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(
            &dir,
            &format!("[users.Alice]\npassword_hash = \"{}\"\n", hash),
        );
        let users = Users::load(&path).unwrap();
        let user = users.verify_password("alice", "pw").unwrap();
        assert_eq!(user.id, "alice");
        assert_eq!(user.username, "Alice");
    }

    #[test]
    fn load_returns_not_found_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let result = Users::load(&dir.path().join("does_not_exist.toml"));
        assert!(matches!(result, Err(UsersError::NotFound(_))));
    }

    #[test]
    fn load_returns_malformed_on_bad_toml() {
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(&dir, "not = toml = at all");
        let result = Users::load(&path);
        assert!(matches!(result, Err(UsersError::Malformed(_))));
    }

    #[test]
    fn load_empty_file_returns_empty_users() {
        let dir = TempDir::new().unwrap();
        let path = write_users_toml(&dir, "");
        let users = Users::load(&path).unwrap();
        assert!(users
            .verify_password("anyone", "pw")
            .is_err());
    }
}
```

- [ ] **Step 5: Run tests, expect all to fail**

Run: `(cd src-tauri && cargo test --lib auth::users)`
Expected: tests panic inside `todo!()`.

- [ ] **Step 6: Implement `hash_password`, `Users::load`, `Users::verify_password`**

Replace the three `todo!()` bodies in `src-tauri/src/auth/users.rs`:

```rust
impl Users {
    pub fn load(path: &Path) -> Result<Self, UsersError> {
        if !path.exists() {
            return Err(UsersError::NotFound(path.display().to_string()));
        }
        let raw = std::fs::read_to_string(path)?;
        let parsed: UsersFile = toml::from_str(&raw)
            .map_err(|e| UsersError::Malformed(e.to_string()))?;

        let mut by_id = HashMap::new();
        let mut display_names = HashMap::new();
        for (name, record) in parsed.users {
            let id = name.to_lowercase();
            display_names.insert(id.clone(), name);
            by_id.insert(id, record);
        }
        Ok(Users { by_id, display_names })
    }

    pub fn verify_password(&self, username: &str, plaintext: &str) -> Result<User, AuthError> {
        let id = username.to_lowercase();
        let record = match self.by_id.get(&id) {
            Some(r) => r,
            None => {
                // Spend the same time as a real verify to keep the
                // unknown-user case from being a timing oracle. argon2
                // verify dominates either branch; this dummy verify
                // costs roughly the same as a real one.
                let dummy_hash = hash_password("dummy-to-avoid-timing-oracle")
                    .unwrap_or_default();
                let _ = PasswordHash::new(&dummy_hash).and_then(|h| {
                    Argon2::default().verify_password(plaintext.as_bytes(), &h)
                });
                return Err(AuthError::InvalidCredentials);
            }
        };

        let parsed = PasswordHash::new(&record.password_hash)
            .map_err(|e| AuthError::Hash(e.to_string()))?;
        Argon2::default()
            .verify_password(plaintext.as_bytes(), &parsed)
            .map_err(|_| AuthError::InvalidCredentials)?;

        let username = self
            .display_names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.clone());
        Ok(User { id, username })
    }
}

pub fn hash_password(plaintext: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(plaintext.as_bytes(), &salt)
        .map_err(|e| AuthError::Hash(e.to_string()))?
        .to_string();
    Ok(hash)
}
```

- [ ] **Step 7: Run the auth tests, expect them to pass**

Run: `(cd src-tauri && cargo test --lib auth::users)`
Expected: all 7 tests pass.

Note: tests run slower because argon2 verification is intentionally CPU-heavy (~100ms each). Allowing 5–10 seconds for the auth test suite is normal.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/auth/ src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(auth): add users.toml loader + argon2 password verify with tests"
```

---

## Task 1.4 — `auth/sessions.rs` — sled-backed sessions

**Files:**
- Create: `src-tauri/src/auth/sessions.rs`
- Modify: `src-tauri/src/auth/mod.rs` (add `pub mod sessions;`)

**Background:** Sessions persist across server restarts. A `Sessions` struct wraps a sled tree at `<data_root>/sessions/`. API: `create(user_id) -> SessionId`, `lookup(id) -> Option<User>`, `delete(id)`, plus background pruning of expired entries.

Session IDs are 32 bytes of OS randomness, encoded as URL-safe base64 (no padding) for use in cookies (~43 chars, no special chars). 30-day cookie expiry — stored as a unix-timestamp expiry in each row.

**API to land:**

```rust
pub struct SessionId(String); // base64url-no-pad of 32 random bytes

pub struct Sessions { /* sled::Db handle */ }

impl Sessions {
    pub fn open(path: &Path) -> Result<Self, SessionError>;
    pub fn create(&self, user_id: &str) -> Result<SessionId, SessionError>;
    pub fn lookup(&self, id: &str) -> Option<String>; // returns user_id if valid + unexpired
    pub fn delete(&self, id: &str) -> Result<(), SessionError>;
    pub fn prune_expired(&self) -> Result<usize, SessionError>;
}
```

`Sessions` is `Clone` (sled handles are cheap to clone — they reference-count internally).

- [ ] **Step 1: Add `base64` dep if not present**

Check `src-tauri/Cargo.toml` — `base64 = "0.22"` is already there. Good.

- [ ] **Step 2: Create the module skeleton**

Modify `src-tauri/src/auth/mod.rs`:

```rust
pub mod sessions;
pub mod users;
```

Create `src-tauri/src/auth/sessions.rs`:

```rust
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use rand::RngCore;
use serde::{Deserialize, Serialize};

const DEFAULT_SESSION_TTL_SECS: u64 = 60 * 60 * 24 * 30; // 30 days

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("sled error: {0}")]
    Sled(#[from] sled::Error),
    #[error("session serde error: {0}")]
    Serde(#[from] bincode::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionId(pub String);

impl SessionId {
    pub fn new() -> Self {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        SessionId(URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionRecord {
    user_id: String,
    expires_at_unix: u64,
}

#[derive(Clone)]
pub struct Sessions {
    db: sled::Db,
    ttl_secs: u64,
}

impl Sessions {
    pub fn open(path: &Path) -> Result<Self, SessionError> {
        todo!()
    }

    pub fn create(&self, user_id: &str) -> Result<SessionId, SessionError> {
        todo!()
    }

    pub fn lookup(&self, id: &str) -> Option<String> {
        todo!()
    }

    pub fn delete(&self, id: &str) -> Result<(), SessionError> {
        todo!()
    }

    pub fn prune_expired(&self) -> Result<usize, SessionError> {
        todo!()
    }

    #[cfg(test)]
    pub(crate) fn with_ttl(mut self, ttl_secs: u64) -> Self {
        self.ttl_secs = ttl_secs;
        self
    }
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}
```

`bincode` is used to serialize `SessionRecord` because it's faster than JSON and sled values are arbitrary bytes. Add `bincode = "1.3"` to `[dependencies]` in `Cargo.toml` if not present (it's not). Run `cargo check`.

- [ ] **Step 3: Write the failing tests**

Append to `src-tauri/src/auth/sessions.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_sessions(dir: &TempDir) -> Sessions {
        Sessions::open(&dir.path().join("sessions")).unwrap()
    }

    #[test]
    fn create_then_lookup_returns_user_id() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir);
        let sid = s.create("alice").unwrap();
        assert_eq!(s.lookup(sid.as_str()), Some("alice".to_string()));
    }

    #[test]
    fn lookup_for_unknown_session_returns_none() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir);
        assert!(s.lookup("nonexistent").is_none());
    }

    #[test]
    fn delete_removes_session() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir);
        let sid = s.create("bob").unwrap();
        s.delete(sid.as_str()).unwrap();
        assert!(s.lookup(sid.as_str()).is_none());
    }

    #[test]
    fn session_ids_are_unique() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir);
        let a = s.create("alice").unwrap();
        let b = s.create("alice").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn sessions_survive_reopen() {
        let dir = TempDir::new().unwrap();
        let sid = {
            let s = open_sessions(&dir);
            s.create("carol").unwrap()
        };
        // sled dropped, reopen the same path
        let s2 = open_sessions(&dir);
        assert_eq!(s2.lookup(sid.as_str()), Some("carol".to_string()));
    }

    #[test]
    fn expired_sessions_are_not_returned_by_lookup() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir).with_ttl(0); // expires immediately
        let sid = s.create("dave").unwrap();
        // sleep a moment to ensure clock advances past expiry
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(s.lookup(sid.as_str()).is_none());
    }

    #[test]
    fn prune_expired_removes_old_rows() {
        let dir = TempDir::new().unwrap();
        let s = open_sessions(&dir).with_ttl(0);
        for u in ["e", "f", "g"] {
            s.create(u).unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        let pruned = s.prune_expired().unwrap();
        assert_eq!(pruned, 3);
    }
}
```

- [ ] **Step 4: Run tests, expect to fail**

Run: `(cd src-tauri && cargo test --lib auth::sessions)`
Expected: panics inside `todo!()`.

- [ ] **Step 5: Implement the methods**

Replace the `todo!()` bodies:

```rust
impl Sessions {
    pub fn open(path: &Path) -> Result<Self, SessionError> {
        let db = sled::open(path)?;
        Ok(Sessions { db, ttl_secs: DEFAULT_SESSION_TTL_SECS })
    }

    pub fn create(&self, user_id: &str) -> Result<SessionId, SessionError> {
        let sid = SessionId::new();
        let record = SessionRecord {
            user_id: user_id.to_string(),
            expires_at_unix: now_unix().saturating_add(self.ttl_secs),
        };
        let bytes = bincode::serialize(&record)?;
        self.db.insert(sid.as_str().as_bytes(), bytes)?;
        self.db.flush()?;
        Ok(sid)
    }

    pub fn lookup(&self, id: &str) -> Option<String> {
        let bytes = self.db.get(id.as_bytes()).ok().flatten()?;
        let record: SessionRecord = bincode::deserialize(&bytes).ok()?;
        if record.expires_at_unix <= now_unix() {
            // Lazy expiry — fire-and-forget removal.
            let _ = self.db.remove(id.as_bytes());
            return None;
        }
        Some(record.user_id)
    }

    pub fn delete(&self, id: &str) -> Result<(), SessionError> {
        self.db.remove(id.as_bytes())?;
        self.db.flush()?;
        Ok(())
    }

    pub fn prune_expired(&self) -> Result<usize, SessionError> {
        let now = now_unix();
        let mut count = 0;
        let mut to_delete = Vec::new();
        for entry in self.db.iter() {
            let (k, v) = entry?;
            if let Ok(record) = bincode::deserialize::<SessionRecord>(&v) {
                if record.expires_at_unix <= now {
                    to_delete.push(k.to_vec());
                }
            }
        }
        for k in to_delete {
            self.db.remove(k)?;
            count += 1;
        }
        self.db.flush()?;
        Ok(count)
    }
}
```

- [ ] **Step 6: Run tests, expect all to pass**

Run: `(cd src-tauri && cargo test --lib auth::sessions)`
Expected: all 7 tests pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/auth/ src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(auth): add sled-backed persisted sessions with 30-day expiry"
```

---

## Task 1.5 — `storage/user_data.rs` — per-user config + chat persistence

**Files:**
- Create: `src-tauri/src/storage/user_data.rs`
- Modify: `src-tauri/src/storage/mod.rs` (add `pub mod user_data;`)

**Background:** Each user has a directory `<data_root>/users/<uid>/` containing `config.json` (LLM provider config, theme, zoom, recently-opened) and `chat/<project_id>/<conversation_id>.json`. This module is the only thing that reads/writes there; the HTTP layer in later phases will call it.

The `UserConfig` shape is intentionally loose at this layer (`serde_json::Value`) because the frontend owns the schema. The server only stores and retrieves blobs; it doesn't interpret them. Type-safe wrappers can be added later if a field becomes server-significant.

`recently_opened` is the one structured field — used by `/auth/whoami` to populate the project picker.

**API to land:**

```rust
pub struct UserData { /* data_root path */ }

impl UserData {
    pub fn new(data_root: PathBuf) -> Self;

    // Config — opaque JSON blob per user.
    pub fn load_config(&self, user_id: &str) -> Result<serde_json::Value, UserDataError>;
    pub fn save_config(&self, user_id: &str, value: &serde_json::Value) -> Result<(), UserDataError>;

    // Recently-opened — separate from the opaque config because the server uses it.
    pub fn recently_opened(&self, user_id: &str) -> Vec<String>;
    pub fn add_recently_opened(&self, user_id: &str, project_id: &str) -> Result<(), UserDataError>;

    // Chat — one file per conversation.
    pub fn list_conversations(&self, user_id: &str, project_id: &str)
        -> Result<Vec<ConversationMeta>, UserDataError>;
    pub fn load_conversation(&self, user_id: &str, project_id: &str, conv_id: &str)
        -> Result<serde_json::Value, UserDataError>;
    pub fn save_conversation(&self, user_id: &str, project_id: &str, conv_id: &str, value: &serde_json::Value)
        -> Result<(), UserDataError>;
}
```

**Atomic writes:** all `save_*` methods write to a temp file then `rename` — crash-safe.

- [ ] **Step 1: Create the module**

Append `pub mod user_data;` to `src-tauri/src/storage/mod.rs`. Create `src-tauri/src/storage/user_data.rs`:

```rust
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum UserDataError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid user_id: {0}")]
    InvalidUserId(String),
    #[error("invalid project_id: {0}")]
    InvalidProjectId(String),
    #[error("invalid conversation_id: {0}")]
    InvalidConversationId(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMeta {
    pub id: String,
    pub modified_unix: u64,
}

#[derive(Debug, Clone)]
pub struct UserData {
    data_root: PathBuf,
}

impl UserData {
    pub fn new(data_root: PathBuf) -> Self {
        UserData { data_root }
    }

    pub fn load_config(&self, user_id: &str) -> Result<serde_json::Value, UserDataError> {
        todo!()
    }

    pub fn save_config(&self, user_id: &str, value: &serde_json::Value) -> Result<(), UserDataError> {
        todo!()
    }

    pub fn recently_opened(&self, user_id: &str) -> Vec<String> {
        todo!()
    }

    pub fn add_recently_opened(&self, user_id: &str, project_id: &str) -> Result<(), UserDataError> {
        todo!()
    }

    pub fn list_conversations(&self, user_id: &str, project_id: &str)
        -> Result<Vec<ConversationMeta>, UserDataError>
    {
        todo!()
    }

    pub fn load_conversation(&self, user_id: &str, project_id: &str, conv_id: &str)
        -> Result<serde_json::Value, UserDataError>
    {
        todo!()
    }

    pub fn save_conversation(&self, user_id: &str, project_id: &str, conv_id: &str, value: &serde_json::Value)
        -> Result<(), UserDataError>
    {
        todo!()
    }
}

// ---- private helpers ----

fn safe_segment(label: &'static str, segment: &str) -> Result<(), UserDataError> {
    // Allow [a-zA-Z0-9._-] only — covers usernames, conv UUIDs, project hashes.
    if segment.is_empty()
        || !segment.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(match label {
            "user_id" => UserDataError::InvalidUserId(segment.into()),
            "project_id" => UserDataError::InvalidProjectId(segment.into()),
            "conversation_id" => UserDataError::InvalidConversationId(segment.into()),
            _ => UserDataError::InvalidUserId(segment.into()),
        });
    }
    Ok(())
}

fn atomic_write_json(path: &Path, value: &serde_json::Value) -> Result<(), UserDataError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(serde_json::to_vec_pretty(value)?.as_slice())?;
        f.sync_all()?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}
```

Note on `project_id` segment safety: in higher layers, `project_id` is the canonical project path. For *filesystem segment* use we must turn it into a safe string — Phase 2 will hash it (e.g. blake3 of canonical path → hex). For now, `safe_segment` enforces the constraint at this layer; the caller is responsible for passing a sanitized id.

- [ ] **Step 2: Write the failing tests**

Append to `src-tauri/src/storage/user_data.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn ud() -> (TempDir, UserData) {
        let dir = TempDir::new().unwrap();
        let ud = UserData::new(dir.path().to_path_buf());
        (dir, ud)
    }

    #[test]
    fn load_config_returns_empty_object_when_missing() {
        let (_dir, ud) = ud();
        let cfg = ud.load_config("alice").unwrap();
        assert!(cfg.is_object());
        assert_eq!(cfg.as_object().unwrap().len(), 0);
    }

    #[test]
    fn save_then_load_config_roundtrip() {
        let (_dir, ud) = ud();
        let value = json!({"llm": {"endpoint": "https://api.example.com", "model": "x"}});
        ud.save_config("alice", &value).unwrap();
        assert_eq!(ud.load_config("alice").unwrap(), value);
    }

    #[test]
    fn save_config_is_atomic_no_leftover_tmp() {
        let (dir, ud) = ud();
        ud.save_config("alice", &json!({"x": 1})).unwrap();
        let entries: Vec<_> = fs::read_dir(dir.path().join("users/alice")).unwrap().collect();
        assert!(!entries.iter().any(|e| {
            e.as_ref().unwrap().path().extension().map(|x| x == "tmp").unwrap_or(false)
        }));
    }

    #[test]
    fn invalid_user_id_is_rejected() {
        let (_dir, ud) = ud();
        let result = ud.save_config("../etc", &json!({}));
        assert!(matches!(result, Err(UserDataError::InvalidUserId(_))));
        let result = ud.load_config("alice/bob");
        assert!(matches!(result, Err(UserDataError::InvalidUserId(_))));
    }

    #[test]
    fn config_isolated_per_user() {
        let (_dir, ud) = ud();
        ud.save_config("alice", &json!({"who": "alice"})).unwrap();
        ud.save_config("bob", &json!({"who": "bob"})).unwrap();
        assert_eq!(ud.load_config("alice").unwrap(), json!({"who": "alice"}));
        assert_eq!(ud.load_config("bob").unwrap(), json!({"who": "bob"}));
    }

    #[test]
    fn recently_opened_starts_empty() {
        let (_dir, ud) = ud();
        assert_eq!(ud.recently_opened("alice"), Vec::<String>::new());
    }

    #[test]
    fn add_recently_opened_dedupes_and_moves_to_front() {
        let (_dir, ud) = ud();
        ud.add_recently_opened("alice", "proj-a").unwrap();
        ud.add_recently_opened("alice", "proj-b").unwrap();
        ud.add_recently_opened("alice", "proj-a").unwrap();
        assert_eq!(
            ud.recently_opened("alice"),
            vec!["proj-a".to_string(), "proj-b".to_string()]
        );
    }

    #[test]
    fn save_then_list_conversation() {
        let (_dir, ud) = ud();
        ud.save_conversation("alice", "proj1", "abc", &json!({"messages": []})).unwrap();
        let conv = ud.load_conversation("alice", "proj1", "abc").unwrap();
        assert_eq!(conv, json!({"messages": []}));
        let list = ud.list_conversations("alice", "proj1").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, "abc");
    }

    #[test]
    fn list_conversations_for_unused_project_is_empty() {
        let (_dir, ud) = ud();
        assert!(ud.list_conversations("alice", "untouched").unwrap().is_empty());
    }
}
```

- [ ] **Step 3: Run tests, expect to fail**

Run: `(cd src-tauri && cargo test --lib storage::user_data)`
Expected: panic inside `todo!()`.

- [ ] **Step 4: Implement the methods**

Replace the `todo!()` bodies:

```rust
impl UserData {
    fn user_dir(&self, user_id: &str) -> Result<PathBuf, UserDataError> {
        safe_segment("user_id", user_id)?;
        Ok(self.data_root.join("users").join(user_id))
    }

    pub fn load_config(&self, user_id: &str) -> Result<serde_json::Value, UserDataError> {
        let path = self.user_dir(user_id)?.join("config.json");
        if !path.exists() {
            return Ok(serde_json::Value::Object(Default::default()));
        }
        let raw = fs::read(&path)?;
        Ok(serde_json::from_slice(&raw)?)
    }

    pub fn save_config(&self, user_id: &str, value: &serde_json::Value) -> Result<(), UserDataError> {
        let path = self.user_dir(user_id)?.join("config.json");
        atomic_write_json(&path, value)
    }

    pub fn recently_opened(&self, user_id: &str) -> Vec<String> {
        let path = match self.user_dir(user_id) {
            Ok(p) => p.join("recently_opened.json"),
            Err(_) => return vec![],
        };
        if !path.exists() {
            return vec![];
        }
        let raw = match fs::read(&path) {
            Ok(b) => b,
            Err(_) => return vec![],
        };
        serde_json::from_slice::<Vec<String>>(&raw).unwrap_or_default()
    }

    pub fn add_recently_opened(&self, user_id: &str, project_id: &str) -> Result<(), UserDataError> {
        let path = self.user_dir(user_id)?.join("recently_opened.json");
        let mut current = self.recently_opened(user_id);
        current.retain(|p| p != project_id);
        current.insert(0, project_id.to_string());
        current.truncate(20);
        atomic_write_json(&path, &serde_json::json!(current))
    }

    fn chat_dir(&self, user_id: &str, project_id: &str) -> Result<PathBuf, UserDataError> {
        safe_segment("user_id", user_id)?;
        safe_segment("project_id", project_id)?;
        Ok(self.data_root.join("users").join(user_id).join("chat").join(project_id))
    }

    pub fn list_conversations(&self, user_id: &str, project_id: &str)
        -> Result<Vec<ConversationMeta>, UserDataError>
    {
        let dir = self.chat_dir(user_id, project_id)?;
        if !dir.exists() {
            return Ok(vec![]);
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let id = match path.file_stem().and_then(|s| s.to_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            let modified_unix = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            out.push(ConversationMeta { id, modified_unix });
        }
        out.sort_by(|a, b| b.modified_unix.cmp(&a.modified_unix));
        Ok(out)
    }

    pub fn load_conversation(&self, user_id: &str, project_id: &str, conv_id: &str)
        -> Result<serde_json::Value, UserDataError>
    {
        safe_segment("conversation_id", conv_id)?;
        let path = self.chat_dir(user_id, project_id)?.join(format!("{conv_id}.json"));
        let raw = fs::read(&path)?;
        Ok(serde_json::from_slice(&raw)?)
    }

    pub fn save_conversation(&self, user_id: &str, project_id: &str, conv_id: &str, value: &serde_json::Value)
        -> Result<(), UserDataError>
    {
        safe_segment("conversation_id", conv_id)?;
        let path = self.chat_dir(user_id, project_id)?.join(format!("{conv_id}.json"));
        atomic_write_json(&path, value)
    }
}
```

- [ ] **Step 5: Run tests, expect all to pass**

Run: `(cd src-tauri && cargo test --lib storage::user_data)`
Expected: all 9 tests pass.

- [ ] **Step 6: Run the full test suite, expect no regressions**

Run: `(cd src-tauri && cargo test --lib)`
Expected: all phase-1 tests pass; any pre-existing Rust tests in the repo still pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/storage/
git commit -m "feat(storage): add per-user data + chat persistence with atomic writes"
```

---

## Phase 1 — Done check

Before moving to Phase 2, verify:

- [ ] `cargo test --lib storage::paths` — all 8 tests pass
- [ ] `cargo test --lib auth::users` — all 7 tests pass
- [ ] `cargo test --lib auth::sessions` — all 7 tests pass
- [ ] `cargo test --lib storage::user_data` — all 9 tests pass
- [ ] `cargo build` — entire crate (including existing Tauri code) builds with no errors
- [ ] No new clippy warnings introduced (run `cargo clippy --lib`, compare to baseline)
- [ ] Branch `feat/browser-lan-port` exists and all four commits land cleanly

---

# Phase 2 — HTTP server skeleton (outline)

**Goal:** Stand up an `axum` server that boots from `main.rs`, opens two listeners, and handles auth (`/auth/login`, `/auth/logout`, `/auth/whoami`) plus a stub SSE endpoint. No business endpoints yet — those land in Phase 4. The existing `api_server.rs` (tiny_http) and `clip_server.rs` keep running unchanged for now; they're removed in Phase 7.

**New Cargo deps:** `axum = "0.8"` (or current stable), `tower = "0.5"`, `tower-http = { version = "0.6", features = ["fs", "trace"] }`, `tower-cookies = "0.10"`, `hyper = "1"`, `rust-embed = "8"`.

**Files to create:**
- `src-tauri/src/http/mod.rs` — router assembly, app-state struct (`Arc<AppState>` holds `Users`, `Sessions`, `UserData`, `paths_config`).
- `src-tauri/src/http/auth.rs` — handlers: `POST /auth/login`, `POST /auth/logout`, `GET /auth/whoami`. Session middleware as a `tower::Layer`.
- `src-tauri/src/http/events.rs` — `GET /events` SSE stream. Registers an `mpsc::Sender<Event>` in a `SessionBus` keyed by session id; held open per connection; auto-removed on drop.
- `src-tauri/src/storage/session_bus.rs` — `SessionBus` = `Arc<Mutex<HashMap<String, mpsc::Sender<Event>>>>`. `register(session_id) -> mpsc::Receiver<Event>`, `send_to(session_id, event)`, `unregister(session_id)`.
- `src-tauri/src/http/error.rs` — uniform `ApiError { code, message, details }` → `axum::response::Response` impl, matching the spec's error shape.
- `src-tauri/src/embed.rs` — `rust-embed` derive bundling `dist/`, fallback handler serves `index.html` for any non-API non-asset path (SPA routing).
- `src-tauri/src/config.rs` — parse env vars / startup TOML into a `ServerConfig { port, projects_root, data_root, legacy_19828_enabled }`.

**Files to modify:**
- `src-tauri/src/main.rs` — load `ServerConfig`, build `AppState`, spawn the two `axum::Server` listeners on the tokio runtime. Keep the existing Tauri bootstrap path under `#[cfg(feature = "tauri-shell")]` if you want to preserve the dual-target dev loop; otherwise hard-fork by replacing `main.rs` and putting Tauri behind a deleted feature flag (it'll be removed in Phase 7 anyway).
- `src-tauri/Cargo.toml` — add the deps above.

**Key behaviors / tests:**
- `POST /auth/login {username, password}` — calls `Users::verify_password`, on success calls `Sessions::create`, returns 200 with `Set-Cookie: session=<id>; HttpOnly; SameSite=Lax; Max-Age=2592000; Path=/`. On failure returns 401 `{error: {code: "INVALID_CREDENTIALS", ...}}`.
- `POST /auth/logout` — reads the session cookie, calls `Sessions::delete`, returns 204 + `Set-Cookie: session=; Max-Age=0`.
- `GET /auth/whoami` — middleware-extracted `User` → returns `{user_id, username, recently_opened: [...]}` (from `UserData::recently_opened`). Without a valid cookie → 401.
- Session middleware: layer that reads the `session` cookie, calls `Sessions::lookup`, on hit injects `User` into request extensions; on miss, leaves it absent. Handlers that need a user use an extractor `User` that returns 401 if missing.
- SSE: opening `/events` registers an mpsc sender in `SessionBus`; sending events from any business path (later phases) routes by `session_id`. For now, this endpoint just opens, holds open, and closes on disconnect with no events flowing.
- Dual listener: `main.rs` calls `axum::serve` twice with the same `Router` — the second one (`127.0.0.1:19828`) uses a router without the auth middleware. Conditional on `LEGACY_19828_ENABLED`.

**Tests required:**
- `http::auth` integration tests using `axum::Router::oneshot` (no real TCP): login happy path, login wrong password, whoami without cookie, whoami with valid cookie, logout invalidates.
- `http::events` smoke test: open SSE, send one event via `SessionBus::send_to`, assert it arrives.
- `http::error` test: error variant → expected JSON body + status code.

**Verifiable by:** `cargo run --bin llm-wiki-server` starts the binary, and `curl http://localhost:8080/api/v1/auth/whoami` returns 401 with the uniform error JSON. `curl -c c.txt -X POST -H 'Content-Type: application/json' -d '{"username":"alice","password":"pw"}' http://localhost:8080/api/v1/auth/login` (against a `users.toml` you've populated by hand for testing) sets a cookie. Subsequent `curl -b c.txt http://localhost:8080/api/v1/auth/whoami` returns 200 with the user.

---

# Phase 3 — Core extraction (outline)

**Goal:** Move business logic out of `#[tauri::command]` wrappers in `src-tauri/src/commands/*.rs` into `src-tauri/src/core/*.rs`. Each Tauri command becomes a 3-line wrapper calling a `core::*` function. No behavior change. The desktop app still runs, all existing Rust tests still pass.

**Why now (Phase 3, not Phase 1):** the new HTTP handlers in Phase 4 will call these `core/` functions directly. Doing the extraction before Phase 4 means Phase 4 handlers don't have to also fight the `AppHandle`-based command shape.

**Files to create:**
- `src-tauri/src/core/mod.rs`
- `src-tauri/src/core/wiki.rs` — page read/write, frontmatter, wikilinks (extracted from `commands/fs.rs` + bits scattered elsewhere)
- `src-tauri/src/core/sources.rs` — ingest orchestration (from `commands/*` and existing pipeline code)
- `src-tauri/src/core/ingest_queue.rs` — persistent serial queue (port the existing implementation; today's queue is in `commands/` and uses `AppHandle::emit`)
- `src-tauri/src/core/search.rs` — hybrid BM25 + vector (from `commands/search.rs`)
- `src-tauri/src/core/graph.rs` — Louvain, 4-signal relevance, insights (currently mostly frontend — move computational parts server-side if any are; check `commands/`)
- `src-tauri/src/core/lint.rs`
- `src-tauri/src/core/extract/mod.rs` — pdfium/calamine/docx; port `commands/extract_images.rs` and related
- `src-tauri/src/core/vectorstore.rs` — port `commands/vectorstore.rs`
- `src-tauri/src/core/llm_client.rs` — HTTP client for OpenAI-compatible endpoints (consolidates whatever exists today across the codebase)
- `src-tauri/src/core/project.rs` — port `commands/project.rs`
- `src-tauri/src/core/event_sink.rs` — the `EventSink` trait used by streaming jobs:

  ```rust
  pub trait EventSink: Send + Sync + 'static {
      fn emit(&self, event_type: &str, payload: serde_json::Value);
  }
  ```

  Implementations: `http::events::SessionEventSink` (sends through `SessionBus`), `core::event_sink::CapturingSink` (test-only `Arc<Mutex<Vec<(String, Value)>>>`).

**Files to modify:**
- Every file under `src-tauri/src/commands/*.rs` — replace each `#[tauri::command]` body with a call into the new `core::*` function. AppHandle usages become explicit parameters. Event emissions become `event_sink.emit(...)` calls.

**Key tests:**
- `core::ingest_queue` against `CapturingSink` — assert phase order (`parse` → `analyze` → `write` → `done`) without HTTP.
- Persistence test: enqueue, drop the queue, reopen, verify `IN_PROGRESS` → `INTERRUPTED`.
- Every extracted module gets ported tests; new modules that wrap existing logic keep parity.

**Verifiable by:** `cargo test --lib core::*` green; desktop app (`npm run tauri dev`) launches and successfully ingests a doc (regression baseline).

---

# Phase 4 — HTTP handlers (outline)

**Goal:** Wire every UI-required endpoint as thin axum handlers over `core/`. The frontend isn't using them yet (Phase 5 does that), but they're independently exercisable via `curl` / integration tests.

**Files to create:** `src-tauri/src/http/{projects,wiki,sources,chat,config,fs_browser,files,proxy,agent_api}.rs`. Each is a Router-returning function mounted in `http::mod`.

**Endpoint map (recap from spec):**

| Path | Handler | Notes |
|---|---|---|
| `GET /projects/list` | `core::project::list_projects(projects_root)` | scans root for valid projects |
| `POST /projects/open` | `core::project::open` after `resolve_under` | records via `UserData::add_recently_opened` |
| `POST /projects/create` | `core::project::create` after `resolve_under` | |
| `GET /wiki/page` | `core::wiki::read_page` | sets `ETag` |
| `PUT /wiki/page` | `core::wiki::write_page` w/ `If-Match` | 412 on mismatch |
| `POST /search` | `core::search::hybrid` | |
| `GET /graph` | `core::graph::for_project` | |
| `POST /sources/ingest` | `core::ingest_queue::enqueue` w/ `SessionEventSink` | returns 202 |
| `GET /sources/list` | `core::sources::list` | |
| `GET /sources/ingest/jobs?mine=true` | filter by `user_id` | |
| `GET /sources/ingest/jobs/<id>` | snapshot for SSE-reconnect catch-up | |
| `GET /chat/conversations?project_id=` | `UserData::list_conversations` | |
| `GET /chat/conversation/<id>` | `UserData::load_conversation` | |
| `POST /chat/send` | enqueues an LLM call streaming via `SessionEventSink`; persists on done | |
| `GET /config` | `UserData::load_config` | |
| `PUT /config` | `UserData::save_config` | |
| `GET /fs/list?path=` | `resolve_under(projects_root, ..)` + `fs::read_dir` | |
| `POST /fs/mkdir` | `resolve_under` + `fs::create_dir_all` | |
| `GET /files/<file-id>/raw` | resolves file-id under current project, streams bytes with content-type guessing | |
| `POST /proxy/llm` | reads user config, forwards request to user's provider, streams response | |
| `/agent/*` | the legacy 19828 handlers, mounted on both listeners | |

**Key tests per handler:**
- One happy-path round-trip.
- One auth-gate test (no cookie → 401).
- One path-scope test per fs-touching endpoint (`..` → 400 `PATH_ESCAPE`).

**Verifiable by:** `cargo test --lib http::*` green; `curl` exercises each endpoint with a valid session cookie.

---

# Phase 5 — Frontend transport rewire (outline)

**Goal:** Strip every `@tauri-apps/*` import. Introduce the two transport modules. Migrate ~25 call sites.

**Files to create:**
- `src/lib/api.ts` — `apiCall<TReq, TRes>(method, path, body?): Promise<TRes>`. Throws `ApiError` (typed). Base URL from `import.meta.env.VITE_API_BASE` (defaults to same-origin so the prod build "just works"). Includes credentials (`credentials: 'include'`) so the session cookie travels.
- `src/lib/events.ts` — singleton `EventSource` to `/api/v1/events`. `subscribe(type: string, handler: (data: unknown) => void): () => void`. Reconnects on drop with exponential backoff (cap ~30s). Multi-subscriber dispatch.
- `src/components/auth/login-view.tsx` — username + password form, POSTs to `/auth/login`, on success calls `whoami` and dispatches to main app.
- `src/components/layout/folder-browser-dialog.tsx` — modal showing tree rooted at projects-root via `/fs/list`. "Create folder" button → `/fs/mkdir`. "Select" returns chosen path.
- `src/components/layout/user-badge.tsx` — top-bar avatar, displays username, dropdown with "Log out" (POSTs `/auth/logout` → reloads).
- `src/components/layout/project-switcher.tsx` — top-bar `📂 <project-name> ▾`, dropdown lists projects from `/projects/list` + recently-opened first.

**Files to modify (high-impact list; the full grep below catches the rest):**

```
src/App.tsx
src/components/settings/settings-view.tsx
src/components/settings/sections/scheduled-import-section.tsx  (delete; out of scope)
src/components/settings/sections/about-section.tsx
src/components/settings/sections/llm-provider-section.tsx
src/components/settings/sections/api-server-section.tsx
src/components/chat/chat-message.tsx
src/components/project/create-project-dialog.tsx
src/components/layout/file-tree.tsx
src/components/layout/update-banner.tsx
src/components/sources/sources-view.tsx
src/components/editor/file-preview.tsx
src/lib/claude-cli-transport.ts        (delete; out of scope)
src/lib/codex-cli-transport.ts         (delete; out of scope)
src/lib/markdown-image-resolver.ts
src/lib/search.ts
src/lib/extract-source-images.ts
src/lib/embedding.ts
src/lib/theme.ts
src/lib/project-store.ts
src/lib/project-identity.ts
src/lib/project-file-sync.ts
src/commands/fs.ts
src/commands/file-sync.ts
```

Use `grep -rn "@tauri-apps" src/` after each batch to find what's left. The phase is done when that grep is empty (except for inside `src/` files explicitly deleted).

**Mechanical replacements (apply consistently):**

| Tauri | Replacement |
|---|---|
| `invoke('foo', {a, b})` | `apiCall('POST', '/foo', {a, b})` (path matches the new HTTP handler) |
| `listen('event', cb)` → `unlisten` | `subscribe('event', cb)` → returned `unsubscribe()` |
| `convertFileSrc(path)` | `\`/api/v1/files/${encodeURIComponent(fileId)}/raw\`` (must use the `file_id`, not raw path) |
| `await open({directory: true})` | `await openFolderBrowser()` (renders `<FolderBrowserDialog>`, resolves to selected path) |
| `await message(text, {kind: 'error'})` | existing toast/dialog primitive |
| `await load('store').get(key)` | `(await apiCall<UserConfig>('GET', '/config'))[key]` |
| `await load('store').set(key, val)` | `apiCall('PUT', '/config', { ...current, [key]: val })` |
| `openUrl(url)` | `window.open(url, '_blank', 'noopener')` |
| `getCurrentWindow().theme()` | `matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'` |
| Tauri-plugin-http LLM calls | `apiCall('POST', '/proxy/llm', ...)` |
| Autostart calls | remove entirely (out of scope) |

**Stores to adapt:**
- `src/stores/*.ts` keep their public shapes. Where they previously persisted via `tauri-plugin-store`, they now persist via `/api/v1/config`. The simplest pattern: each store reads its slice from `config.json` on init, debounces writes back.

**Tests:**
- `src/lib/api.test.ts` — mocked `fetch`, error shape parsing, cookie credential mode, JSON serialization.
- `src/lib/events.test.ts` — mocked `EventSource`, type routing, multi-subscriber, reconnect.
- Existing component tests now mock `api.ts` instead of `invoke()`.

**Verifiable by:** `vitest` green; `grep -rn "@tauri-apps" src/` returns nothing (except in files explicitly being removed in Phase 7); `npm run build` produces `dist/` with no Tauri-specific output.

---

# Phase 6 — New UI screens & settings reorganization (outline)

**Goal:** First time the app is end-to-end usable in a browser.

**Files to create (or finish if started in Phase 5):**
- `src/components/auth/login-view.tsx`
- `src/components/layout/folder-browser-dialog.tsx`
- `src/components/layout/user-badge.tsx`
- `src/components/layout/project-switcher.tsx`

**Files to modify:**
- `src/App.tsx` — top-level: render `<LoginView>` when `whoami` is 401, else render the existing app shell with `<UserBadge>` + `<ProjectSwitcher>` in the top bar.
- `src/components/settings/settings-view.tsx` — reorder existing sections into the Personal / Project grouping per the spec; add section header components with the helper text ("only you see these", "shared with everyone").
- `src/components/project/create-project-dialog.tsx` — uses `<FolderBrowserDialog>` instead of `@tauri-apps/plugin-dialog`.
- `src/components/sources/sources-view.tsx` — same.

**Manual smoke test for this phase:**
1. Run `cargo run --bin llm-wiki-server` with a populated `users.toml` and a `projects_root` containing one existing project.
2. Open `http://localhost:<port>` in a browser.
3. Verify the login screen appears.
4. Log in as the user. Verify the project auto-opens, top bar shows username + project name.
5. Open Settings, verify Personal and Project sections render correctly.
6. Open a wiki page; verify file preview works.
7. Switch projects via the top-bar switcher.
8. Click "Create new project" → folder browser opens → select / create a folder → new project opens.

---

# Phase 7 — Cleanup (outline)

**Goal:** Remove the Tauri shell entirely. Final commit makes the codebase Tauri-free.

**Tasks:**
1. **Rename `src-tauri/` → `src-server/`.** Update `Cargo.toml` workspace pointers if any. Update `package.json` script paths.
2. **Delete Tauri-specific files** inside `src-server/`:
   - `tauri.conf.json`, `tauri.linux.conf.json`, `tauri.macos.conf.json`, `tauri.windows.conf.json`
   - `capabilities/`
   - `windows-app-manifest.xml`
   - `build.rs` (if only used for Tauri; check first — if it does something else useful keep that part)
   - Any `gen/` directory under `src-tauri/`
   - The `icons/` directory if not used by anything else (keep one for the binary's own icon if applicable)
3. **Delete Tauri-specific Rust modules** that have no use post-port:
   - `src-server/src/api_server.rs` (replaced by the new axum agent_api handlers on 127.0.0.1:19828)
   - `src-server/src/clip_server.rs` (Web Clipper is out of scope)
   - `src-server/src/tray.rs` (no native tray in a server)
   - `src-server/src/panic_guard.rs` if Tauri-specific (check; the underlying panic-recovery idea might still be useful inside the axum handler wrapper)
   - `src-server/src/commands/` (all moved to `core/` in Phase 3; these are now empty shells)
   - The legacy `lib.rs` re-exports of Tauri-specific things
4. **Update `Cargo.toml`:**
   - Remove `tauri`, `tauri-build`, `tauri-plugin-*` deps.
   - Remove the `[build-dependencies]` block if only `tauri-build`.
   - Rename the package + binary from `llm-wiki` → `llm-wiki-server`.
5. **Update `package.json`:**
   - Remove `@tauri-apps/*` from `dependencies` and `devDependencies`.
   - Remove the `tauri` script.
   - Update `build` to `vite build` (no typecheck-then-tauri).
6. **Update `vite.config.ts`:**
   - Remove `TAURI_DEV_HOST` handling.
   - Keep the host/port config but bind to `0.0.0.0` in dev so other devices on the LAN can hit Vite during dev too.
   - Add a `server.proxy` rule routing `/api/*` to the Rust server's dev port so the dev loop is `npm run dev` + `cargo run --bin llm-wiki-server` separately.
7. **Run final checks:**
   - `cargo build --release` produces `target/release/llm-wiki-server` and nothing else
   - `npm run build` produces `dist/` with no `@tauri-apps` references
   - `grep -rn "@tauri-apps\|tauri::\|#\[tauri" .` returns nothing in `src/` or `src-server/src/` (allowed: `Cargo.lock`, archive folders)
8. **Write `plans/manual-test-plan.md`** — the acceptance checklist for the merge:
   - Cold-boot the server with empty data_root + a fresh `users.toml`
   - Log in / out
   - Auto-open most recent project on subsequent login
   - Create a project, open it, ingest a small PDF, see progress streamed
   - Read the resulting wiki page; verify wikilinks resolve
   - Search hits the new page
   - Two users in two browsers (or two incognito windows) — Alice ingests, Bob refreshes and sees the result
   - Bob's chat history isn't visible to Alice
   - Cross-browser sanity: Chrome + Safari minimum
9. **Run the manual test plan.** Document any failures and fix before merging.
10. **Merge `feat/browser-lan-port` into `main`.**

---

# Self-review (run before declaring this plan done)

**Spec coverage** — every spec section can point to a task or phase outline:

- Goal / non-goals → Plan header
- Decision 1 (drop Tauri) → Phase 7
- Decision 2 (big-bang on a branch) → Phase 0 branch creation + Phase 7 merge
- Decision 3 (single binary, rust-embed) → Phase 2 `embed.rs` + Phase 7 build scripts
- Decision 4 (axum) → Phase 2
- Decision 5 (two listeners) → Phase 2 `main.rs` + Phase 4 `agent_api` handler
- Decision 6 (per-user accounts) → Task 1.3 + Phase 2 auth handlers
- Decision 7 (persisted sessions, sled) → Task 1.4
- Decision 8 (shared per-project, per-user else) → Task 1.5 + Phase 3 (project handling) + Phase 4 (handler scopes)
- Decision 9 (per-session SSE, no broadcast) → Phase 2 `events.rs` + Phase 3 `EventSink` + Phase 4 use sites
- Decision 10 (hybrid folder model) → Task 1.2 + Phase 4 `fs_browser`
- Decision 11 (server-side LLM/embedding proxy) → Phase 4 `proxy` handler
- Decision 12 (scope cuts) → Phase 5 file deletions + Phase 7 cleanup
- Path safety chokepoint → Task 1.2
- Uniform error format → Phase 2 `http::error`
- ETag concurrency → Phase 4 wiki PUT
- Long-running jobs / interrupt recovery → Phase 3 ingest queue port + Phase 4 jobs endpoints
- SSE disconnect handling → Phase 2 SSE wrapper + Phase 5 `events.ts`
- File perms `0700`/`0600` on data_root → Phase 2 server boot (create data_root with restrictive perms)
- Server config (env / TOML) → Phase 2 `config.rs`
- `users.toml` admin CLI → Phase 7 (CLI subcommand of the same binary: `llm-wiki-server user add alice`)
- Frontend changes table → Phase 5
- New screens → Phase 6
- Settings reorganization → Phase 6
- API surface → Phase 4
- Testing strategy → tests embedded throughout, with `manual-test-plan.md` listed in Phase 7

**Placeholder scan** — none. Phase 1 has full TDD steps; Phases 2-7 are outlines (intentional, with rationale called out in "How this plan is scoped").

**Type consistency** — `User`, `UserData`, `Sessions`, `Users`, `EventSink`, `ApiError`, `ConversationMeta` shapes are defined once (in Phase 1 or in Phase 2/3 outlines) and referenced consistently in later phases.

**Scope** — Phase 1 is sized to land in a couple of days of focused work; Phases 2-7 are each phase-sized, not plan-sized. After Phase 1 lands, write the detailed plan for Phase 2.
