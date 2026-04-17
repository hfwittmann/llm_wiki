/**
 * Sweep pending review items and auto-resolve those whose underlying
 * condition has been addressed by subsequent ingests.
 *
 * Triggered when the ingest queue drains.
 * Conservative: only auto-resolves high-certainty cases. Preserves
 * contradiction / suggestion / confirm types that need human judgment.
 */

import { listDirectory, readFile } from "@/commands/fs"
import { useReviewStore, type ReviewItem } from "@/stores/review-store"
import { useActivityStore } from "@/stores/activity-store"
import type { FileNode } from "@/types/wiki"
import { normalizePath } from "@/lib/path-utils"

// ── Helpers ────────────────────────────────────────────────────────────────

function flattenMdFiles(nodes: FileNode[]): FileNode[] {
  const files: FileNode[] = []
  for (const node of nodes) {
    if (node.is_dir && node.children) {
      files.push(...flattenMdFiles(node.children))
    } else if (!node.is_dir && node.name.endsWith(".md")) {
      files.push(node)
    }
  }
  return files
}

/** Build an index of wiki pages: id (filename without .md) + title → normalized */
async function buildWikiIndex(projectPath: string): Promise<{
  byId: Set<string>
  byTitle: Set<string>
}> {
  const pp = normalizePath(projectPath)
  const byId = new Set<string>()
  const byTitle = new Set<string>()

  try {
    const tree = await listDirectory(`${pp}/wiki`)
    const files = flattenMdFiles(tree)

    for (const file of files) {
      const id = file.name.replace(/\.md$/, "").toLowerCase()
      byId.add(id)

      // Also capture frontmatter title for fuzzy matching
      try {
        const content = await readFile(file.path)
        const match = content.match(/^---\n[\s\S]*?^title:\s*["']?(.+?)["']?\s*$/m)
        if (match) {
          byTitle.add(match[1].trim().toLowerCase())
        }
      } catch {
        // skip unreadable files
      }
    }
  } catch {
    // no wiki directory yet
  }

  return { byId, byTitle }
}

/**
 * Extract candidate page names from a review item's title / description.
 * Conservative — only flags items where we can confidently identify a page name.
 */
function extractCandidateNames(item: ReviewItem): string[] {
  const names = new Set<string>()

  // The review title itself is often the missing page name
  // e.g. "Missing page: 注意力机制" or "注意力机制"
  const cleaned = item.title
    .replace(/^(missing[\s-]?page[:：]\s*|缺失页面[:：]\s*|缺少页面[:：]\s*)/i, "")
    .trim()

  if (cleaned && cleaned.length <= 100) {
    names.add(cleaned.toLowerCase())
  }

  // Also check affectedPages — these reference files directly
  for (const page of item.affectedPages ?? []) {
    const base = page.split("/").pop()?.replace(/\.md$/, "")
    if (base) names.add(base.toLowerCase())
  }

  return Array.from(names)
}

/** Check if a candidate name matches an existing wiki page */
function pageExists(name: string, index: { byId: Set<string>; byTitle: Set<string> }): boolean {
  const normalized = name.trim().toLowerCase()
  if (!normalized) return false

  // Exact filename match (kebab-case or matching existing id)
  if (index.byId.has(normalized)) return true
  if (index.byId.has(normalized.replace(/\s+/g, "-"))) return true

  // Exact title match (from frontmatter)
  if (index.byTitle.has(normalized)) return true

  return false
}

// ── Main ──────────────────────────────────────────────────────────────────

/**
 * Scan pending review items and auto-resolve those whose condition
 * no longer holds. Called when the ingest queue drains.
 */
export async function sweepResolvedReviews(projectPath: string): Promise<number> {
  const store = useReviewStore.getState()
  const pending = store.items.filter((i) => !i.resolved)

  if (pending.length === 0) return 0

  const index = await buildWikiIndex(projectPath)

  let resolvedCount = 0

  for (const item of pending) {
    // Only auto-resolve types where we can make a high-certainty judgment
    if (item.type === "missing-page") {
      const names = extractCandidateNames(item)
      if (names.length > 0 && names.some((n) => pageExists(n, index))) {
        store.resolveItem(item.id, "auto-resolved")
        resolvedCount++
      }
    } else if (item.type === "duplicate") {
      // If any affected page no longer exists, the duplicate situation changed —
      // auto-resolve (user or cascade-delete took care of it).
      const affected = item.affectedPages ?? []
      if (affected.length > 0) {
        const allStillExist = affected.every((p) => {
          const base = p.split("/").pop()?.replace(/\.md$/, "").toLowerCase()
          return base ? index.byId.has(base) : false
        })
        if (!allStillExist) {
          store.resolveItem(item.id, "auto-resolved")
          resolvedCount++
        }
      }
    }
    // contradiction / suggestion / confirm → keep, need human judgment
  }

  if (resolvedCount > 0) {
    // Log to activity panel so user sees it happened
    useActivityStore.getState().addItem({
      type: "query",
      title: "Review cleanup",
      status: "done",
      detail: `Auto-resolved ${resolvedCount} stale review item${resolvedCount > 1 ? "s" : ""}`,
      filesWritten: [],
    })
    console.log(`[Sweep Reviews] Auto-resolved ${resolvedCount} review items`)
  }

  return resolvedCount
}
