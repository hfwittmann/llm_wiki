# Phase 5 frontend migration audit

Generated for the Phase-5 transport rewire. Lists every file with `@tauri-apps/*`
imports and the planned replacement strategy. Use this as the checklist for
Tasks 5.3–5.10.

| File | Tauri imports | Replacement (Phase 5 task) |
|---|---|---|
| src/App.tsx | invoke, plugin-dialog open, plugin-autostart | apiCall (5.4), FolderBrowserDialog (5.7), [delete] (5.2) |
| src/commands/file-sync.ts | invoke | apiCall to /sources/* (5.3) |
| src/commands/fs.ts | invoke | apiCall + fileRawUrl (5.3, 5.8) |
| src/components/chat/chat-message.tsx | plugin-opener openUrl | window.open (5.8) |
| src/components/editor/file-preview.tsx | convertFileSrc | fileRawUrl (5.8) |
| src/components/layout/file-tree.tsx | plugin-dialog message | existing toast/dialog (5.8) |
| src/components/layout/update-banner.tsx | plugin-opener openUrl | window.open (5.8) |
| src/components/project/create-project-dialog.tsx | plugin-dialog open | FolderBrowserDialog (5.7) |
| src/components/settings/sections/about-section.tsx | plugin-opener openUrl | window.open (5.8) |
| src/components/settings/sections/api-server-section.tsx | plugin-opener openUrl | window.open (5.8) |
| src/components/settings/sections/llm-provider-section.tsx | invoke | apiCall (5.4) |
| src/components/settings/sections/scheduled-import-section.tsx | plugin-dialog open | DELETE — out of v1 scope (5.2) |
| src/components/settings/settings-view.tsx | invoke, plugin-autostart | apiCall, [delete] (5.4, 5.2) |
| src/components/sources/sources-view.tsx | plugin-dialog open | FolderBrowserDialog (5.7) |
| src/lib/claude-cli-transport.ts | invoke, listen | DELETE — out of v1 scope (5.2) |
| src/lib/codex-cli-transport.ts | invoke, listen | DELETE — out of v1 scope (5.2) |
| src/lib/embedding.ts | invoke | apiCall to /proxy/llm (5.3, 5.9) |
| src/lib/extract-source-images.ts | invoke | apiCall — needs server endpoint (5.3) |
| src/lib/markdown-image-resolver.ts | convertFileSrc | fileRawUrl (5.3) |
| src/lib/project-file-sync.ts | listen | subscribe (5.5) |
| src/lib/project-identity.ts | plugin-store load | user-config (5.6) |
| src/lib/project-store.ts | plugin-store load | user-config (5.6) |
| src/lib/search.ts | invoke | apiCall to /search (5.3) |
| src/lib/tauri-fetch.ts | plugin-http | DELETE (callers use /proxy/llm) (5.9) |
| src/lib/theme.ts | api/window getCurrentWindow | matchMedia (5.8) |

Notes on differences from the Phase-4 audit starter:
- `src/lib/llm-providers.ts` references `@tauri-apps` only in comments, not in imports — **not listed**.
- `src/components/settings/sections/api-server-section.tsx` uses `openUrl` (plugin-opener), not `invoke` — updated accordingly.

## Tests with @tauri-apps mocks

Test files that mock Tauri APIs will need updates to mock fetch/EventSource
instead, OR be marked stale and rewritten:

- src/commands/fs.test.ts
- src/lib/__tests__/claude-cli-transport.test.ts (delete with claude-cli-transport)
- src/lib/codex-cli-transport.test.ts (delete with codex-cli-transport)
- src/lib/embedding.test.ts
- src/lib/embedding.real-llm.test.ts
- src/lib/llm-client.real-llm.test.ts
- src/lib/markdown-image-resolver.test.ts
- src/lib/project-file-sync.test.ts
- src/lib/search-rrf.test.ts
- src/lib/search.scenarios.test.ts
- src/lib/tauri-fetch.test.ts (delete with tauri-fetch.ts)
- src/lib/vision-caption.real-llm.test.ts
- src/lib/vision.real-llm.test.ts
