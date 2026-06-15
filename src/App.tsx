import { useState, useEffect } from "react"
import i18n from "@/i18n"
import { useWikiStore } from "@/stores/wiki-store"
import { useReviewStore } from "@/stores/review-store"
import { useLintStore } from "@/stores/lint-store"
import { useChatStore } from "@/stores/chat-store"
import { BASE_FONT_SIZE_PX, useZoomStore } from "@/stores/zoom-store"
import { listDirectory, openProject } from "@/commands/fs"
import { getLastProject, saveLastProject, loadLlmConfig, loadLanguage, loadSearchApiConfig, loadEmbeddingConfig, loadMineruConfig, loadMultimodalConfig, loadOutputLanguage, loadProviderConfigs, loadActivePresetId, loadProxyConfig, loadSourceWatchConfig, loadApiConfig, loadGeneralConfig, loadZoomLevel } from "@/lib/project-store"
import { loadReviewItems, loadLintItems, loadChatHistory } from "@/lib/persist"
import { setupAutoSave } from "@/lib/auto-save"
import { AppLayout } from "@/components/layout/app-layout"
import { WelcomeScreen } from "@/components/project/welcome-screen"
import { CreateProjectDialog } from "@/components/project/create-project-dialog"
import { OpenProjectDialog } from "@/components/project/open-project-dialog"
import type { WikiProject } from "@/types/wiki"
import { LoginView } from "@/components/auth/login-view"
import type { AuthUser } from "@/components/auth/login-view"
import { apiCall, ApiError } from "@/lib/api"
import { setAuthUser } from "@/lib/auth"

function applyDocumentZoom(level: number) {
  document.documentElement.style.fontSize = `${BASE_FONT_SIZE_PX * level}px`
}

function App() {
  const project = useWikiStore((s) => s.project)
  const setProject = useWikiStore((s) => s.setProject)
  const setFileTree = useWikiStore((s) => s.setFileTree)
  const setSelectedFile = useWikiStore((s) => s.setSelectedFile)
  const setActiveView = useWikiStore((s) => s.setActiveView)
  const zoomLevel = useZoomStore((s) => s.level)
  const [showCreateDialog, setShowCreateDialog] = useState(false)
  const [showOpenDialog, setShowOpenDialog] = useState(false)
  const [loading, setLoading] = useState(true)
  // auth: null = checking, false = unauthenticated, AuthUser = authenticated
  const [authUser, setAuthUser_] = useState<AuthUser | null | false>(null)

  // Set up auto-save once on mount. The Tauri-era clip watcher (polling the
  // local Web Clipper daemon on :19827) is not part of the browser/LAN build.
  useEffect(() => {
    setupAutoSave()
  }, [])

  useEffect(() => {
    // Apply interface zoom globally, including welcome/settings screens. We
    // scale the rem base instead of using transform: scale() so layout and
    // pointer coordinates remain native; fixed-pixel panels keep their caps.
    applyDocumentZoom(zoomLevel)
  }, [zoomLevel])

  // Dev-only helper for visually testing the update-banner UX.
  // Open dev tools and run:
  //   __llmwiki_testUpdateBanner()
  // to inject a fake "available" result into the update store —
  // banner appears at the top + red dot lights up the gear icon.
  // Run again with arg `false` (or call setDismissed via the store)
  // to clear. Gated on `import.meta.env.DEV` so the helper never
  // ships in production builds.
  useEffect(() => {
    if (!import.meta.env.DEV) return
    ;(async () => {
      const storeMod = await import("@/stores/update-store")
      const { useUpdateStore } = storeMod
      // Expose the live store getter on window so you can inspect
      // state from devtools when debugging banner behavior.
      ;(window as unknown as { __llmwiki_updateStore?: typeof useUpdateStore }).__llmwiki_updateStore = useUpdateStore
      ;(window as unknown as { __llmwiki_testUpdateBanner?: (clear?: boolean) => void }).__llmwiki_testUpdateBanner = (clear = false) => {
        if (clear) {
          useUpdateStore.getState().setResult(
            { kind: "up-to-date", local: __APP_VERSION__, remote: __APP_VERSION__ },
            Date.now(),
          )
          useUpdateStore.getState().setDismissed(null)
          console.log("[test] update banner cleared")
          return
        }
        useUpdateStore.getState().setResult(
          {
            kind: "available",
            local: __APP_VERSION__,
            remote: "v999.0.0",
            release: {
              name: "v999.0.0 (test)",
              tag_name: "v999.0.0",
              body:
                "Test release for banner-UX verification.\n\n" +
                "- Bigger red dot on the Settings icon\n" +
                "- Top banner with one-click dismiss\n" +
                "- Once dismissed, won't reappear for this version",
              html_url: "https://github.com/nashsu/llm_wiki/releases",
              published_at: new Date().toISOString(),
            },
          },
          Date.now(),
        )
        useUpdateStore.getState().setDismissed(null)
        console.log(
          "[test] update banner injected. Run __llmwiki_testUpdateBanner(true) to clear.",
        )
      }
    })()
  }, [])

  // Background update check is disabled in the browser/LAN fork: the
  // checker hits GitHub Releases for `nashsu/llm_wiki`, which is the
  // upstream Tauri desktop app — not this fork. The 403 it returned
  // on every boot was just GitHub rate-limiting an unauthenticated
  // request to a repo we don't track. If a fork-level update
  // mechanism is ever added, wire it here.
  useEffect(() => {
    let cancelled = false
    const timer = setTimeout(async () => {
      if (cancelled) return
      return // disabled for browser/LAN fork; see comment above
      try {
        const { loadUpdateCheckState, saveUpdateCheckState } = await import(
          "@/lib/project-store"
        )
        const { useUpdateStore } = await import("@/stores/update-store")
        const { checkForUpdates, UPDATE_CHECK_CACHE_MS } = await import(
          "@/lib/update-check"
        )

        const persisted = await loadUpdateCheckState()
        if (persisted) useUpdateStore.getState().hydrate(persisted)

        const state = useUpdateStore.getState()
        if (!state.enabled) {
          console.log("[update-check] skipped: user disabled auto-check in settings")
          return
        }

        const now = Date.now()
        // Cache hit requires BOTH the timestamp AND the in-memory
        // result to be present. `lastCheckedAt` is persisted to
        // disk but `lastResult` deliberately is not — keeping the
        // GitHub payload out of the persisted store keeps disk
        // size + privacy footprint small. The downside: a fresh
        // cold start has `lastResult === null` even when
        // `lastCheckedAt` is recent, in which case we MUST refetch
        // — otherwise we'd skip the check AND have no result to
        // display, leaving the banner permanently stuck off.
        // (This was the user-reported bug: "kind=none, no banner".)
        const fresh =
          state.lastCheckedAt !== null &&
          state.lastResult !== null &&
          now - state.lastCheckedAt < UPDATE_CHECK_CACHE_MS
        if (fresh) {
          const ageMin = Math.round((now - (state.lastCheckedAt ?? 0)) / 60_000)
          console.log(
            `[update-check] skipped: cache hit (last check ${ageMin} min ago, ` +
              `cache window ${UPDATE_CHECK_CACHE_MS / 60_000} min). ` +
              `Last result: kind=${state.lastResult?.kind ?? "none"}`,
          )
          return
        }

        useUpdateStore.getState().setChecking(true)
        console.log(
          `[update-check] fetching GitHub releases (local=${__APP_VERSION__})`,
        )
        const result = await checkForUpdates({
          currentVersion: __APP_VERSION__,
          repo: "nashsu/llm_wiki",
        })
        if (cancelled) return
        useUpdateStore.getState().setResult(result, Date.now())
        if (result.kind === "available") {
          console.log(
            `[update-check] update available: local=${result.local} → remote=${result.remote}`,
          )
        } else if (result.kind === "up-to-date") {
          console.log(
            `[update-check] up to date: local=${result.local}, remote latest=${result.remote}`,
          )
        } else {
          console.log(`[update-check] error: ${result.message}`)
        }
        await saveUpdateCheckState({
          enabled: useUpdateStore.getState().enabled,
          lastCheckedAt: Date.now(),
          dismissedVersion: useUpdateStore.getState().dismissedVersion,
        })
      } catch {
        // Silent — Settings → About lets the user retry manually.
      }
    }, 1500)
    return () => {
      cancelled = true
      clearTimeout(timer)
    }
  }, [])

  // Auto-open last project on startup
  useEffect(() => {
    async function init() {
      try {
        // ── Auth gate ─────────────────────────────────────────────────────────
        // Check if the user is already authenticated before loading any project
        // state.  On 401 we render LoginView and stop here.
        try {
          const user = await apiCall<AuthUser>("GET", "/api/v1/auth/whoami")
          setAuthUser_(user)
          setAuthUser(user)
        } catch (err) {
          if (err instanceof ApiError && err.isUnauthenticated) {
            setAuthUser_(false)
            setLoading(false)
            return
          }
          // Non-auth error (server down, network issue) — proceed anyway;
          // individual API calls will fail and surface their own errors.
          setAuthUser_(false)
          setLoading(false)
          return
        }
        // ─────────────────────────────────────────────────────────────────────

        const savedZoom = await loadZoomLevel()
        applyDocumentZoom(savedZoom)
        useZoomStore.getState().setLevel(savedZoom)

        const savedConfig = await loadLlmConfig()
        if (savedConfig) {
          useWikiStore.getState().setLlmConfig(savedConfig)
        }
        const savedProviderConfigs = await loadProviderConfigs()
        if (savedProviderConfigs) {
          useWikiStore.getState().setProviderConfigs(savedProviderConfigs)
        }
        const savedActivePreset = await loadActivePresetId()
        if (savedActivePreset) {
          useWikiStore.getState().setActivePresetId(savedActivePreset)
          // Re-resolve the active preset's LlmConfig from (preset defaults
          // + saved overrides). Without this, preset default updates
          // (e.g. a corrected Anthropic model ID shipped in a release)
          // never reach users who are relying on defaults — their stored
          // `llmConfig` snapshot from a previous launch would keep the
          // old value. Overrides still win, so an explicit user choice
          // is preserved.
          const { LLM_PRESETS } = await import("@/components/settings/llm-presets")
          const { resolveConfig } = await import("@/components/settings/preset-resolver")
          const preset = LLM_PRESETS.find((p) => p.id === savedActivePreset)
          if (preset) {
            const currentFallback = useWikiStore.getState().llmConfig
            const override = (savedProviderConfigs ?? {})[savedActivePreset]
            const resolved = resolveConfig(preset, override, currentFallback)
            useWikiStore.getState().setLlmConfig(resolved)
            const { saveLlmConfig } = await import("@/lib/project-store")
            await saveLlmConfig(resolved)
          }
        }
        const savedSearchConfig = await loadSearchApiConfig()
        if (savedSearchConfig) {
          useWikiStore.getState().setSearchApiConfig(savedSearchConfig)
        }
        const savedEmbeddingConfig = await loadEmbeddingConfig()
        if (savedEmbeddingConfig) {
          useWikiStore.getState().setEmbeddingConfig(savedEmbeddingConfig)
        }
        const savedMultimodalConfig = await loadMultimodalConfig()
        if (savedMultimodalConfig) {
          useWikiStore.getState().setMultimodalConfig(savedMultimodalConfig)
        }

        const savedMineruConfig = await loadMineruConfig()
        if (savedMineruConfig) {
          useWikiStore.getState().setMineruConfig(savedMineruConfig)
        }
        const savedProxy = await loadProxyConfig()
        if (savedProxy) {
          useWikiStore.getState().setProxyConfig(savedProxy)
        }
        // Local HTTP API server config — global (single token + enable
        // flag for the whole install, not per-project). The Rust side
        // reads `apiConfig.{enabled,token,mcpEnabled}` from `app-state.json`
        // directly; this only hydrates the Zustand store so the
        // Settings UI reflects the persisted values.
        const savedApi = await loadApiConfig()
        if (savedApi) {
          useWikiStore.getState().setApiConfig({
            enabled: typeof savedApi.enabled === "boolean" ? savedApi.enabled : true,
            allowUnauthenticated:
              typeof savedApi.allowUnauthenticated === "boolean"
                ? savedApi.allowUnauthenticated
                : false,
            mcpEnabled:
              typeof savedApi.mcpEnabled === "boolean"
                ? savedApi.mcpEnabled
                : false,
            token: typeof savedApi.token === "string" ? savedApi.token : "",
          })
        }
        const savedGeneral = await loadGeneralConfig()
        useWikiStore.getState().setGeneralConfig(savedGeneral)
        // set_close_behavior was a Tauri-only IPC call; no HTTP equivalent.
        // In the browser/LAN context the close behavior is a no-op.
        const savedLang = await loadLanguage()
        if (savedLang) {
          await i18n.changeLanguage(savedLang)
        }
        const lastProject = await getLastProject()
        if (lastProject) {
          try {
            const proj = await openProject(lastProject.path)
            await handleProjectOpened(proj)
          } catch {
            // Last project no longer valid
          }
        }
      } catch {
        // ignore init errors
      } finally {
        setLoading(false)
      }
    }
    init()
  }, [])

  async function handleProjectOpened(proj: WikiProject) {
    // Flush the OUTGOING project's review/lint/chat state to disk and suspend
    // auto-save before reset empties the stores — otherwise the debounced
    // writers would persist empty arrays back over the old project's pending
    // review / deep-research items.
    const { runWithSuspendedAutoSave } = await import("@/lib/auto-save")
    await runWithSuspendedAutoSave(async () => {
      // Clear all per-project state BEFORE loading new project data
      // to prevent cross-project contamination. MUST be awaited so the
      // ingest queue / graph cache are actually cleared before the new
      // project's state is populated.
      const { resetProjectState } = await import("@/lib/reset-project-state")
      await resetProjectState()

      setProject(proj)
      // Per-project override takes precedence; otherwise fall back to the
      // user's global setting, and only then to "auto". Without the global
      // fallback, opening a project that never had a per-project override
      // saved would silently reset the language to auto-detect — which
      // mis-fires on Latin-script source documents that happen to contain
      // a single stray foreign-language diacritic.
      const projectOutputLang = await loadOutputLanguage(proj.id)
      const globalOutputLang = await loadOutputLanguage()
      useWikiStore.getState().setOutputLanguage(
        projectOutputLang ?? globalOutputLang ?? "auto",
      )
      setSelectedFile(null)
      setActiveView("wiki")
      // Bump data version so any cached graphs/views invalidate
      useWikiStore.getState().bumpDataVersion()
      await saveLastProject(proj)

      // Restore ingest queue (resume interrupted tasks). Keyed by the
      // project's stable UUID so the queue still finds the right project
      // even if the filesystem path changed since the task was enqueued.
      // Await this before starting file sync: watcher events for raw/sources
      // may enqueue ingest tasks and require an active project queue.
      try {
        const { restoreQueue } = await import("@/lib/ingest-queue")
        await restoreQueue(proj.id, proj.path)
      } catch (err) {
        console.error("Failed to restore ingest queue:", err)
      }
      // Same handshake for the dedup-merge queue.
      import("@/lib/dedup-queue").then(({ restoreQueue }) => {
        restoreQueue(proj.id, proj.path).catch((err) =>
          console.error("Failed to restore dedup queue:", err)
        )
      })
      // Start project source watch if enabled
      import("@/lib/project-file-sync").then(async ({ startProjectFileSync, stopProjectFileSync }) => {
        const config = await loadSourceWatchConfig(proj.id)
        useWikiStore.getState().setSourceWatchConfig(config)
        if (config.enabled) {
          startProjectFileSync(proj, config).catch((err) =>
            console.error("Failed to start project file sync:", err)
          )
        } else {
          stopProjectFileSync().catch(() => {})
        }
      }).catch((err) => console.error("Failed to configure project file sync:", err))
      try {
        const tree = await listDirectory(proj.path)
        setFileTree(tree)
      } catch (err) {
        console.error("Failed to load file tree:", err)
      }
      // Load persisted review items
      try {
        const savedReview = await loadReviewItems(proj.path)
        if (savedReview.length > 0) {
          useReviewStore.getState().setItems(savedReview)
        }
      } catch {
        // ignore, start fresh
      }
      // Load persisted lint items
      useLintStore.getState().setItems([])
      try {
        const savedLint = await loadLintItems(proj.path)
        useLintStore.getState().setItems(savedLint)
      } catch {
        useLintStore.getState().setItems([])
      }
      // Load persisted chat history
      try {
        const savedChat = await loadChatHistory(proj.path)
        if (savedChat.conversations.length > 0) {
          useChatStore.getState().setConversations(savedChat.conversations)
          useChatStore.getState().setMessages(savedChat.messages)
          // Set most recent conversation as active
          const sorted = [...savedChat.conversations].sort((a, b) => b.updatedAt - a.updatedAt)
          if (sorted[0]) {
            useChatStore.getState().setActiveConversation(sorted[0].id)
          }
        }
      } catch {
        // ignore, start fresh
      }
    }, () => {
      // If project loading fails after resetProjectState() and before persisted
      // review/lint/chat state has been restored, do not leave auto-save armed
      // against a half-loaded project with empty stores.
      setProject(null)
      setFileTree([])
      setSelectedFile(null)
    })
  }

  async function handleSelectRecent(proj: WikiProject) {
    try {
      const validated = await openProject(proj.path)
      await handleProjectOpened(validated)
    } catch (err) {
      window.alert(`Failed to open project: ${err}`)
    }
  }

  function handleOpenProject() {
    setShowOpenDialog(true)
  }

  async function handleOpenDialogSelect(path: string) {
    try {
      const proj = await openProject(path)
      await handleProjectOpened(proj)
    } catch (err) {
      window.alert(`Failed to open project: ${err}`)
    }
  }

  async function handleSwitchProject() {
    // Flush outgoing project's review/lint/chat to disk and suspend auto-save
    // before reset empties the stores. resumeAutoSave() runs when the next
    // project opens via handleProjectOpened.
    const { flushAndSuspendAutoSave } = await import("@/lib/auto-save")
    await flushAndSuspendAutoSave()

    // Clear all per-project state BEFORE flipping back to the welcome screen
    // so old data cannot leak in via any async render pass.
    const { resetProjectState } = await import("@/lib/reset-project-state")
    await resetProjectState()
    setProject(null)
    setFileTree([])
    setSelectedFile(null)
  }

  function handleLogin(user: AuthUser) {
    setAuthUser_(user)
    setAuthUser(user)
  }

  if (loading || authUser === null) {
    return (
      <div className="flex h-full items-center justify-center bg-background text-muted-foreground">
        Loading...
      </div>
    )
  }

  // Not authenticated — show login gate
  if (authUser === false) {
    return <LoginView onLogin={handleLogin} />
  }

  if (!project) {
    return (
      <>
        <WelcomeScreen
          onCreateProject={() => setShowCreateDialog(true)}
          onOpenProject={handleOpenProject}
          onSelectProject={handleSelectRecent}
        />
        <CreateProjectDialog
          open={showCreateDialog}
          onOpenChange={setShowCreateDialog}
          onCreated={handleProjectOpened}
        />
        <OpenProjectDialog
          open={showOpenDialog}
          onClose={() => setShowOpenDialog(false)}
          onSelect={handleOpenDialogSelect}
        />
      </>
    )
  }

  return (
    <>
      <AppLayout onSwitchProject={handleSwitchProject} />
      <CreateProjectDialog
        open={showCreateDialog}
        onOpenChange={setShowCreateDialog}
        onCreated={handleProjectOpened}
      />
    </>
  )
}

export default App
