# Browser/LAN GUI Phase 3 — Core Extraction (Implementation Plan)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Phase goal:** Move business logic out of `#[tauri::command]` wrappers in `src-tauri/src/commands/*.rs` into a new pure-Rust `src-tauri/src/core/` module tree. Each Tauri command shrinks to a 3-line wrapper that calls into `core::*`. **No behavior change.** The desktop Tauri app still runs and ingests/searches normally at the end of every task. By the end of Phase 3 the codebase is shaped so that Phase 4 can mount HTTP handlers directly over `core::*` with the same level of abstraction.

**Architecture:**
- `core/` is a pure-Rust module tree with no axum, no Tauri, no `AppHandle`, no `#[tauri::command]`. It depends only on the storage/auth modules from Phase 1 and on plain Rust ecosystem crates (pdfium, lancedb, calamine, reqwest, etc.).
- Functions that produce streaming events take an `EventSink` trait parameter. `EventSink` lives in `core::events`. Two implementations: a `TauriEventSink` (in `commands/`) that wraps `AppHandle`, and a `SessionEventSink` (in `http/events.rs`, Phase 4) that targets a `SessionBus` session.
- A `NullEventSink` (in `core::events`) is a no-op implementation used by tests and by HTTP request flows that don't care about streamed progress.
- Where current Tauri command files contain a mix of pure logic + Tauri-specific glue, the pure parts move to `core/`, the Tauri-specific bits stay in `commands/` as thin wrappers. Where a command is *entirely* Tauri-specific (CLI subprocess for claude/codex, dialog opener), it stays in `commands/` and is not exposed via HTTP in Phase 4 — it's out of v1 scope and gets deleted in Phase 7.

**Source spec:** `plans/2026-06-14-browser-lan-gui-design.md` (sections 3 and 4).
**Source plan:** `plans/2026-06-14-browser-lan-gui-implementation.md` (Phase 3 outline section).
**Carryover bugs from Phase 2:** `plans/phase-3-pre-phase-4-bugs.md` — fix before Phase 4 starts, NOT during Phase 3.

**Branch:** Continue on `feat/browser-lan-port`.

**Environment:** macOS dev. `cargo` at `~/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo`. Prefix non-interactive shells:
```
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
```

---

## What's in scope for Phase 3 extraction

| Existing file | Lines | Cmds | AppHandle/Emit | Extract → | Notes |
|---|---|---|---|---|---|
| `commands/vectorstore.rs` | 1071 | 11 | 0 / 0 | `core/vectorstore.rs` | Pure LanceDB wrapper. Mechanical move. |
| `commands/search.rs` | 1414 | 1 | 0 / 0 | `core/search.rs` | Hybrid BM25+vector. Mechanical move. |
| `commands/extract_images.rs` | 972 | 4 | 0 / 0 | `core/extract/` | pdfium/Office image extraction. Mechanical move (split into submodules along existing internal boundaries). |
| `commands/fs.rs` | 2177 | 16 | 1 / 0 | `core/wiki.rs` + `core/files.rs` + `core/fs_ops.rs` | Wiki page CRUD + frontmatter + wikilinks; raw file IO; misc helpers. The one AppHandle use is `tauri::async_runtime::spawn_blocking` which translates to `tokio::task::spawn_blocking`. |
| `commands/project.rs` | 328 | 3 | 2 / 0 | `core/project.rs` (+ keep `open_project_folder` Tauri-only) | `create_project` and `open_project` are pure; `open_project_folder` (opens Finder/Explorer at a path) is Tauri-shell-only and stays in `commands/`. |
| `commands/file_sync.rs` | 1810 | 6 | 11 / 12 | `core/file_sync.rs` (with `EventSink` parameter) | The complex one. EventSink trait abstracts emit. Project file watcher + ingest queue live together. |

## Out of scope for Phase 3 (stays as-is, deleted in Phase 7)

| File | Reason |
|---|---|
| `commands/claude_cli.rs` | CLI subprocess transport. OUT of v1. |
| `commands/codex_cli.rs` | CLI subprocess transport. OUT of v1. |
| `commands/cli_resolver.rs` | Helper for above two. OUT of v1. |
| `clip_server.rs` | Chrome web clipper. OUT of v1. |
| `api_server.rs` | Legacy `127.0.0.1:19828` listener (tiny_http). Phase 4's `legacy_router` will absorb this; until then the desktop binary keeps its own copy. **Do not touch in Phase 3.** |
| `proxy.rs` | HTTP proxy used by frontend to bypass webview CORS. Phase 4's `/proxy/llm` replaces this; Tauri version stays for Tauri compat through Phase 6. |
| `tray.rs` | Tauri system tray. Deleted in Phase 7. |
| `panic_guard.rs` | Tauri-specific. Phase 4 may borrow concepts but Phase 3 leaves it alone. |
| Small lib.rs commands (`clip_server_status`, `api_server_status`, `set_proxy_env`, `set_close_behavior`, `mcp_server_entry_path`, `api_server_reload_config`) | Tauri-shell-only metadata commands. Leave alone. |

## Validation gate per task

Each task ends with:
1. `cargo test --lib` green — no regressions.
2. `cargo build --bin llm-wiki` succeeds — the Tauri desktop binary still compiles.
3. `cargo build --bin llm-wiki-server` succeeds — the Phase-2 HTTP binary still compiles (it doesn't use `core/` yet, but the workspace must be coherent).

Optional manual check between tasks: launch `npm run tauri dev`, perform one ingest of a small PDF, confirm the wiki page appears. This catches behavior changes the type checker missed.

---

## Phase 3 task overview

| # | Task | Outcome |
|---|---|---|
| 3.1 | `core::events::EventSink` trait + Null/Tauri impls | Trait + adapters in place. No business logic moved yet. |
| 3.2 | Extract `vectorstore` | `core/vectorstore.rs` lifted from `commands/vectorstore.rs`; old file becomes thin wrappers. |
| 3.3 | Extract `search` | `core/search.rs` lifted; thin wrapper remains in `commands/search.rs`. |
| 3.4 | Extract `extract_images` | `core/extract/` lifted; submodules split by source format. |
| 3.5 | Extract `fs` page/file ops | `core/wiki.rs` + `core/files.rs` + `core/fs_ops.rs` lifted; `commands/fs.rs` becomes 16 thin wrappers. |
| 3.6 | Extract `project` (excluding `open_project_folder`) | `core/project.rs` lifted; Tauri-only `open_project_folder` stays. |
| 3.7 | Extract `file_sync` with `EventSink` | `core/file_sync.rs` + `core/ingest_queue.rs` lifted; events go through `EventSink`. |
| 3.8 | Add `core::llm_client::LlmClient` (HTTP) | Consolidated reqwest-based OpenAI-compatible client extracted from scattered call sites. Phase 4 will use this directly. |
| 3.9 | Phase 3 done-check + plan-and-go for Phase 4 | Full test suite green; both binaries build; one desktop-app manual smoke; commit a brief postmortem of any surprises in plan. |

---

# Task 3.1 — `core::events::EventSink` trait

**Files:**
- Create: `src-tauri/src/core/mod.rs`
- Create: `src-tauri/src/core/events.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod core;`)
- Modify: `src-tauri/src/commands/mod.rs` — add a `tauri_event_sink.rs` submodule and declare it.
- Create: `src-tauri/src/commands/tauri_event_sink.rs`

**Background:** Every other Phase-3 task will use this trait. Define it once, here, with two reference implementations (`NullEventSink` for tests / no-op callers, and `TauriEventSink` so the existing desktop commands keep emitting events).

**API:**

```rust
pub trait EventSink: Send + Sync {
    fn emit(&self, event_type: &str, payload: serde_json::Value);
}

#[derive(Default, Clone)]
pub struct NullEventSink;
impl EventSink for NullEventSink {
    fn emit(&self, _: &str, _: serde_json::Value) {}
}
```

```rust
// in commands/tauri_event_sink.rs
pub struct TauriEventSink {
    pub app: tauri::AppHandle,
}
impl crate::core::events::EventSink for TauriEventSink {
    fn emit(&self, event_type: &str, payload: serde_json::Value) {
        use tauri::Emitter;
        let _ = self.app.emit(event_type, payload);
    }
}
```

- [ ] **Step 1: Create `core/mod.rs` and `core/events.rs`**

Create `src-tauri/src/core/mod.rs`:

```rust
//! Pure-Rust business logic. No axum, no Tauri, no AppHandle.
//!
//! Functions in this module are called by:
//! - `src-tauri/src/commands/*.rs` (Tauri command wrappers, for the desktop app)
//! - `src-tauri/src/http/*.rs` (axum handlers, for the browser/LAN app, Phase 4)
//!
//! Streamed events (ingest progress, file-watcher updates, LLM tokens) are
//! emitted via the `EventSink` trait in `core::events`.

pub mod events;
```

Create `src-tauri/src/core/events.rs`:

```rust
//! Streaming-event abstraction used by long-running `core::*` operations.
//!
//! `core` code does not know whether events go to Tauri's IPC bridge,
//! an HTTP SSE stream, or get dropped on the floor. It just calls
//! `sink.emit(event_type, payload)` and moves on.

use serde_json::Value;

/// Receives streamed events from `core::*` functions.
///
/// Implementations must be `Send + Sync` and cheap to clone, because the
/// same sink may be shared across spawned tasks within a single operation
/// (e.g., a parallel ingest pipeline).
pub trait EventSink: Send + Sync {
    fn emit(&self, event_type: &str, payload: Value);
}

/// Drop every event. Useful in tests and in HTTP request flows that don't
/// care about streamed progress (e.g., a simple JSON-response handler that
/// just wants the final result).
#[derive(Debug, Clone, Default)]
pub struct NullEventSink;

impl EventSink for NullEventSink {
    fn emit(&self, _event_type: &str, _payload: Value) {}
}

/// Capture events to an in-memory `Vec`. Used by `cfg(test)` callers that
/// want to assert the event stream.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct CapturingEventSink {
    pub events: parking_lot::Mutex<Vec<(String, Value)>>,
}

#[cfg(test)]
impl EventSink for CapturingEventSink {
    fn emit(&self, event_type: &str, payload: Value) {
        self.events.lock().push((event_type.to_string(), payload));
    }
}

#[cfg(test)]
impl CapturingEventSink {
    pub fn snapshot(&self) -> Vec<(String, Value)> {
        self.events.lock().clone()
    }
}
```

- [ ] **Step 2: Declare `core` in `lib.rs`**

Add `pub mod core;` to `src-tauri/src/lib.rs` (alongside the other `pub mod` declarations from Phase 1/2).

- [ ] **Step 3: Create the Tauri adapter**

Create `src-tauri/src/commands/tauri_event_sink.rs`:

```rust
//! Adapter that lets `core::*` functions emit events through Tauri's IPC.

use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::core::events::EventSink;

#[derive(Clone)]
pub struct TauriEventSink {
    pub app: AppHandle,
}

impl TauriEventSink {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl EventSink for TauriEventSink {
    fn emit(&self, event_type: &str, payload: Value) {
        let _ = self.app.emit(event_type, payload);
    }
}
```

Modify `src-tauri/src/commands/mod.rs` — add `pub mod tauri_event_sink;` alongside the existing module declarations.

- [ ] **Step 4: Write tests**

Append to `src-tauri/src/core/events.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn null_sink_swallows_events() {
        let sink = NullEventSink;
        sink.emit("anything", json!({"x": 1}));
        // No way to observe — that's the point. Just verify it compiles
        // and doesn't panic.
    }

    #[test]
    fn capturing_sink_records_events_in_order() {
        let sink = CapturingEventSink::default();
        sink.emit("first", json!({"i": 1}));
        sink.emit("second", json!({"i": 2}));
        let snap = sink.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].0, "first");
        assert_eq!(snap[1].0, "second");
        assert_eq!(snap[0].1, json!({"i": 1}));
    }

    #[test]
    fn sinks_are_send_sync() {
        fn check<T: Send + Sync>() {}
        check::<NullEventSink>();
        check::<CapturingEventSink>();
    }
}
```

- [ ] **Step 5: Run tests, expect 3 pass**

```bash
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH" && (cd src-tauri && cargo test --lib core::events)
```

- [ ] **Step 6: Verify both binaries still build**

```bash
export PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
(cd src-tauri && cargo build --bin llm-wiki && cargo build --bin llm-wiki-server)
```

Expected: both succeed.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/core/ src-tauri/src/commands/tauri_event_sink.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs
git commit -m "feat(core): add EventSink trait with Null, Capturing, and Tauri adapters"
```

---

# Generic extraction pattern (Tasks 3.2 — 3.6, also 3.7 but with EventSink)

These tasks all follow the same pattern. The plan template is here once; individual tasks below pin down the file paths and any per-file gotchas.

**The pattern, applied to a single command file `commands/X.rs`:**

1. **Read the existing file.** Identify three categories of code within it:
   - **Pure functions / data structures / helpers** — no AppHandle, no `#[tauri::command]`, no Tauri imports. These are the "core" parts.
   - **Tauri command wrappers** — functions annotated with `#[tauri::command]`. These usually do parameter validation, call into pure helpers, return `Result<T, String>` (the type Tauri requires).
   - **Tauri-specific glue** — `tauri::async_runtime::spawn_blocking`, `AppHandle`, `Emitter`, plugin-store access. Most files have little to none.

2. **Create the new file `core/X.rs`** containing all category-1 code plus the *bodies* of category-2 functions (rewritten to take plain parameters instead of `AppHandle`, and to return `Result<T, CoreError>` instead of `Result<T, String>`). Errors get a typed `XError` enum (one per module) using `thiserror`. `tauri::async_runtime::spawn_blocking` becomes `tokio::task::spawn_blocking`.

3. **Rewrite `commands/X.rs`** as a thin wrapper. Each old command becomes:

   ```rust
   #[tauri::command]
   pub async fn original_name(
       /* same param shape as before */
   ) -> Result<ReturnType, String> {
       crate::core::x::original_name(/* forward args */)
           .await
           .map_err(|e| e.to_string())
   }
   ```

   Roughly 3–6 lines per command.

4. **Move tests.** Any `#[cfg(test)] mod tests` block in `commands/X.rs` moves to `core/X.rs` (since the logic moved there). Tauri-integration-style tests, if any, stay in `commands/X.rs` (rare in this codebase).

5. **Run validation:**
   - `cargo test --lib core::x` — module-level tests green.
   - `cargo test --lib` — full suite green.
   - `cargo build --bin llm-wiki` succeeds (Tauri binary still compiles).
   - `cargo build --bin llm-wiki-server` succeeds.

6. **Commit** with the message `refactor(core): extract X from commands::X`.

**Per-file gotchas** are listed in the individual task sections below.

---

# Task 3.2 — Extract `vectorstore`

**Files to create:** `src-tauri/src/core/vectorstore.rs`
**Files to modify:** `src-tauri/src/commands/vectorstore.rs` (becomes thin wrappers), `src-tauri/src/core/mod.rs` (add `pub mod vectorstore;`)

**Per-file gotchas:** None known — 1071 lines, 11 commands, no AppHandle, no Emitter. Pure LanceDB wrapper.

**Acceptance:**
- All 11 `commands::vectorstore::*` functions still exist as `#[tauri::command]` wrappers.
- `core::vectorstore::*` exposes the same 11 functions as plain `async fn` returning `Result<T, VectorstoreError>`.
- A new `VectorstoreError` enum with `thiserror::Error` derive covers the error categories (sled errors, lance errors, invalid args, IO).
- `cargo test --lib core::vectorstore` green; existing tests preserved.
- Tauri binary boots; vector search and upsert still work in the desktop app (manual smoke).

**Steps:** Follow the generic pattern above.

**Commit:** `refactor(core): extract vectorstore from commands::vectorstore`

---

# Task 3.3 — Extract `search`

**Files to create:** `src-tauri/src/core/search.rs`
**Files to modify:** `src-tauri/src/commands/search.rs`, `src-tauri/src/core/mod.rs`

**Per-file gotchas:**
- 1414 lines, 1 command (`search_project`). The bulk of the file is helpers and tests.
- `search_project` internally calls `commands::vectorstore::*` functions for vector search. After Task 3.2, those internal calls need to route through `core::vectorstore::*` instead. **Do not call back into `commands::vectorstore` from `core::search`** — that creates a layering inversion. Have the core function take the vectorstore handle directly, or call `core::vectorstore::*` async functions.

**Acceptance:**
- `core::search::search_project` and helpers exist as plain functions / data types.
- `commands::search::search_project` becomes a 3-line wrapper.
- `cargo test --lib core::search` green.
- Hybrid search still works in the desktop app (manual smoke: open project, search, verify results match pre-extraction baseline).

**Commit:** `refactor(core): extract search from commands::search`

---

# Task 3.4 — Extract `extract_images`

**Files to create:** `src-tauri/src/core/extract/mod.rs`, `src-tauri/src/core/extract/pdf.rs`, `src-tauri/src/core/extract/office.rs`
**Files to modify:** `src-tauri/src/commands/extract_images.rs`, `src-tauri/src/core/mod.rs`

**Per-file gotchas:**
- 972 lines, 4 commands. Already mixes PDF and Office in one file; opportunity to split.
- The `extract_*_cmd` and `extract_and_save_*_cmd` variants both exist. Keep both behaviors.
- pdfium initialization may have static state — verify it works equivalently after the move.

**Acceptance:**
- `core::extract::pdf::extract_images(path) -> Result<Vec<ImageBytes>, ExtractError>` and Office equivalent exist.
- `core::extract::pdf::extract_and_save_images(path, dest) -> Result<Vec<ImageMeta>, ExtractError>` and Office equivalent exist.
- The 4 commands in `commands/extract_images.rs` become 3-line wrappers.
- `cargo test --lib core::extract` green.
- Manual smoke: PDF image extraction during ingest still works in the desktop app.

**Commit:** `refactor(core): extract extract_images into core::extract`

---

# Task 3.5 — Extract `fs` (page/file ops)

**Files to create:** `src-tauri/src/core/wiki.rs`, `src-tauri/src/core/files.rs`, `src-tauri/src/core/fs_ops.rs`
**Files to modify:** `src-tauri/src/commands/fs.rs`, `src-tauri/src/core/mod.rs`

**Per-file gotchas:**
- 2177 lines, 16 commands. The largest file.
- The 16 commands split conceptually into:
  - **Wiki-page-level** (read/write wiki markdown, frontmatter, wikilinks, find related pages): `find_related_wiki_pages`, page-write helpers. → `core::wiki`
  - **File-level IO** (read/write/copy/delete arbitrary files, base64, MD5, file metadata): `read_file`, `write_file`, `write_file_base64`, `write_file_atomic`, `copy_file`, `copy_directory`, `delete_file`, `file_exists`, `get_file_modified_time`, `get_file_size`, `get_file_md5`, `read_file_as_base64`. → `core::files`
  - **Directory / preprocessing**: `list_directory`, `preprocess_file`, `create_directory`. → `core::fs_ops`
- The one AppHandle-mentioning line is `tauri::async_runtime::spawn_blocking(move || { ... })`. Replace with `tokio::task::spawn_blocking(move || { ... }).await.map_err(|e| ...)?`.
- Be careful that the 3-way split doesn't break internal helper calls. Read the file once end-to-end before starting to verify the split lines up with actual code clusters.

**Acceptance:**
- 16 thin wrappers remain in `commands/fs.rs`.
- 3 new core modules exist with clear responsibilities.
- `cargo test --lib core::{wiki,files,fs_ops}` green.
- Manual smoke: open project, edit a wiki page, save — content roundtrips.

**Commit:** `refactor(core): split fs.rs into core::{wiki,files,fs_ops}`

---

# Task 3.6 — Extract `project` (excluding folder-opener)

**Files to create:** `src-tauri/src/core/project.rs`
**Files to modify:** `src-tauri/src/commands/project.rs`, `src-tauri/src/core/mod.rs`

**Per-file gotchas:**
- 328 lines, 3 commands. `create_project`, `open_project` — pure. `open_project_folder(app: AppHandle, path: String)` — uses `tauri_plugin_opener::OpenerExt::opener().open_path()` to open the OS file explorer. **This command stays in `commands/project.rs` and is NOT moved to core.** It is Tauri-shell-specific; in the browser model, the equivalent UX is "click a link" in the user's browser.

**Acceptance:**
- `core::project::{create_project, open_project}` exist as plain async functions.
- `commands::project::{create_project, open_project}` become 3-line wrappers.
- `commands::project::open_project_folder` is unchanged.
- `cargo test --lib core::project` green.

**Commit:** `refactor(core): extract project (excluding open_project_folder) into core::project`

---

# Task 3.7 — Extract `file_sync` with `EventSink`

**Files to create:** `src-tauri/src/core/file_sync.rs`, `src-tauri/src/core/ingest_queue.rs`
**Files to modify:** `src-tauri/src/commands/file_sync.rs`, `src-tauri/src/core/mod.rs`

**Per-file gotchas:**
- 1810 lines, 6 commands, **12 emit() calls**. The complex one.
- The 12 emit calls all go through two helper functions (`emit_queue`, `emit_changed_batch`) which take `&AppHandle`. Refactor: those two helpers move into `core::file_sync` and take `&dyn EventSink` instead of `&AppHandle`.
- The 6 commands all take `AppHandle` as first arg in order to construct the helpers. In the new world they take a `&impl EventSink`. The Tauri wrapper constructs a `TauriEventSink` and forwards.
- The file also contains the per-project ingest queue (`enqueue_rescan_changes`, `enqueue_rescan_changes_for_prefixes`, `enqueue_paths` — see line 610+). This is the ingest queue mentioned in the design spec. Split it into `core/ingest_queue.rs` to match the spec's module layout.
- The `EVENT_QUEUE_UPDATED` and `EVENT_CHANGED` event-type constants move to `core::file_sync` so the EventSink callers know the canonical names.

**Step pattern (custom for this task, beyond the generic):**

After the standard extraction, the 6 Tauri commands look like:

```rust
#[tauri::command]
pub async fn start_project_file_watcher(
    app: AppHandle,
    project_id: String,
    project_root: String,
) -> Result<(), String> {
    let sink = crate::commands::tauri_event_sink::TauriEventSink::new(app);
    crate::core::file_sync::start_project_file_watcher(&project_id, &project_root, &sink)
        .await
        .map_err(|e| e.to_string())
}
```

**Acceptance:**
- `core::file_sync` and `core::ingest_queue` exist with the same observable behavior.
- All 6 `commands::file_sync::*` are 3–6-line Tauri wrappers that construct `TauriEventSink`.
- A `cargo test --lib core::file_sync` test exercises the event sequence using `CapturingEventSink` (this is the highest-value new test in Phase 3 — assert the event order parse → analyze → write → done).
- Manual smoke: open project, watcher detects new file, queue progresses, UI updates.

**Commit:** `refactor(core): extract file_sync into core with EventSink abstraction`

---

# Task 3.8 — `core::llm_client::LlmClient`

**Files to create:** `src-tauri/src/core/llm_client.rs`
**Files to modify:** `src-tauri/src/core/mod.rs`

**Background:** Currently there is no single Rust-side LLM HTTP client. The desktop app makes LLM calls from the frontend (via `@tauri-apps/plugin-http` with CORS-bypass headers) — see `src-tauri/src/proxy.rs` for the proxy that intermediates those calls. Phase 4's `/proxy/llm` endpoint will need a server-side OpenAI-compatible HTTP client. Build it now so Phase 4 has a clean target.

**API:**

```rust
pub struct ProviderConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub extra_headers: std::collections::HashMap<String, String>,
}

pub struct LlmClient { /* reqwest::Client */ }

impl LlmClient {
    pub fn new() -> Self;

    /// Non-streaming completion. Body shape is OpenAI-compatible JSON;
    /// we don't introspect it, we just forward.
    pub async fn chat_completion(
        &self,
        cfg: &ProviderConfig,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, LlmError>;

    /// Streaming completion. Each delta (the SSE `data: { … }` payload from
    /// the upstream) is forwarded to `sink.emit("chat:token", payload)`.
    /// Returns when upstream sends `[DONE]` or closes the connection.
    pub async fn chat_completion_stream(
        &self,
        cfg: &ProviderConfig,
        body: serde_json::Value,
        sink: &impl EventSink,
    ) -> Result<(), LlmError>;
}

pub enum LlmError {
    Network(reqwest::Error),
    UpstreamStatus { status: u16, body: String },
    Timeout,
    InvalidConfig(String),
}
```

**Steps:**

- [ ] Create `core/llm_client.rs` with the structs and `impl LlmClient`. Use `reqwest::Client` (already in Cargo.toml).
- [ ] Implement `chat_completion` as a single POST + JSON parse.
- [ ] Implement `chat_completion_stream` parsing SSE-style upstream lines: `data: {...}` → forward payload; `data: [DONE]` → terminate.
- [ ] Tests: use `mockito` (add as `[dev-dependencies]` if not present) to stand up a fake OpenAI-compatible endpoint and assert:
  - Non-streaming: 200 with JSON body roundtrips.
  - Non-streaming: 401 → `UpstreamStatus { status: 401, .. }`.
  - Streaming: 3 deltas → 3 `chat:token` events captured.
  - Streaming: `[DONE]` terminates cleanly.
  - Invalid `base_url` → `InvalidConfig`.
- [ ] No production caller wires this in yet — Phase 4 does that.

**Commit:** `feat(core): add LlmClient for OpenAI-compatible HTTP and SSE streaming`

---

# Task 3.9 — Phase 3 done-check

**Files to modify:** none mandatory; may add a brief `plans/phase-3-summary.md` if surprises were significant.

- [ ] **Full test suite green:** `cargo test --lib` (expect 186 + ~30–50 new from extractions = ~220 ± 30).
- [ ] **Both binaries build cleanly** (no warnings introduced by Phase 3).
- [ ] **Desktop app manual smoke:**
  - `npm run tauri dev` launches.
  - Open an existing project.
  - Ingest one small PDF (the watcher fires, the queue progresses, the wiki page appears).
  - Search returns expected results.
  - Edit a wiki page, save, reload — content roundtrips.
  - No new errors in dev tools console.
- [ ] **`llm-wiki-server` smoke unchanged:** the server binary still passes the Phase-2 curl flow.
- [ ] **Postmortem** (optional but encouraged): if any task surprised the implementer (longer than expected, hidden coupling, mismatch with the plan), capture it in `plans/phase-3-summary.md` so Phase 4's plan can budget realistically.
- [ ] **Pre-Phase-4 bug log** (`plans/phase-3-pre-phase-4-bugs.md`) still tracks the two items from Phase 2; **do not yet resolve them** — Phase 4's first task handles them.

---

## What Phase 4 will need from Phase 3

Phase 4 mounts axum handlers under `/projects`, `/wiki`, `/sources`, `/chat`, `/config`, `/fs`, `/files`, `/proxy/llm`. Each handler will:

1. Take `axum::extract::State<AppState>` and `AuthUser` (for protected routes).
2. Call into `core::*` — never into `commands::*`.
3. For streaming endpoints, pass a `SessionEventSink` (Phase 4 will write this in `http::events`) that forwards events to the requester's SSE stream via `SessionBus`.
4. Convert `core::*` error enums into `ApiError` with appropriate codes.

For Phase 4 to do that cleanly, Phase 3's `core::*` modules must:
- Be importable from `http::*` (they are — both live under `src-tauri/src`).
- Accept all input as plain types — no `AppHandle`, no `tauri::*`. Verified by Phase 3 done-check.
- Return typed error enums, not `Result<T, String>`. Verified by Phase 3 done-check.
- Stream events through `EventSink` so the HTTP `SessionEventSink` plugs in trivially. Verified by Phase 3 done-check.

If any of those four properties has slipped during Phase 3, Phase 4's first task is to fix them. Better to catch in 3.9 than mid-Phase-4.
