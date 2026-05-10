import { invoke } from "@tauri-apps/api/core"
import {
  copyFile,
  deleteFile,
  findRelatedWikiPages,
  listDirectory,
  preprocessFile,
  readFile,
  writeFile,
} from "@/commands/fs"
import type { WikiProject, FileNode } from "@/types/wiki"
import type { LlmConfig } from "@/stores/wiki-store"
import { enqueueBatch, enqueueIngest } from "@/lib/ingest-queue"
import { hasUsableLlm } from "@/lib/has-usable-llm"
import { getFileName, getFileStem, normalizePath } from "@/lib/path-utils"
import { decidePageFate } from "@/lib/source-delete-decision"
import {
  parseFrontmatterArray,
  parseSources,
  writeFrontmatterArray,
  writeSources,
} from "@/lib/sources-merge"
import { removeFromIngestCache } from "@/lib/ingest-cache"
import { removePageEmbedding } from "@/lib/embedding"
import {
  buildDeletedKeys,
  cleanIndexListing,
  stripDeletedWikilinks,
} from "@/lib/wiki-cleanup"
import { collectAllFilesIncludingDot } from "@/lib/sources-tree-delete"

export const INGESTABLE_SOURCE_EXTENSIONS = new Set([
  "md",
  "mdx",
  "txt",
  "pdf",
  "docx",
  "pptx",
  "xlsx",
  "xls",
  "csv",
  "json",
  "html",
  "htm",
  "rtf",
  "xml",
  "yaml",
  "yml",
])

export interface DeleteSourceResult {
  deletedWikiPaths: string[]
  rewrittenSourcePages: number
}

export interface DeleteSourceFolderResult {
  deletedWikiPaths: string[]
}

export function isIngestableSourcePath(path: string): boolean {
  const fileName = normalizePath(path).split("/").pop() ?? ""
  if (!fileName || fileName.startsWith(".")) return false
  const ext = fileName.includes(".") ? fileName.split(".").pop()?.toLowerCase() : ""
  return ext ? INGESTABLE_SOURCE_EXTENSIONS.has(ext) : false
}

export function folderContextForSourcePath(sourcePath: string, sourcesRoot = "raw/sources"): string {
  const path = normalizePath(sourcePath)
  const root = normalizePath(sourcesRoot)
  const rel = path.startsWith(`${root}/`) ? path.slice(root.length + 1) : path
  const parts = rel.split("/")
  parts.pop()
  return parts.join(" > ")
}

export async function enqueueSourceIngest(
  project: WikiProject,
  sourcePaths: string[],
  llmConfig: LlmConfig,
  options: { sourceRoot?: string; rootContext?: string } = {},
): Promise<string[]> {
  if (!hasUsableLlm(llmConfig)) return []
  const files = sourcePaths
    .filter(isIngestableSourcePath)
    .map((sourcePath) => ({
      sourcePath,
      folderContext: withRootContext(
        folderContextForSourcePath(sourcePath, options.sourceRoot),
        options.rootContext,
      ),
    }))
  if (files.length === 0) return []
  return enqueueBatch(project.id, files)
}

export async function importSourceFiles(
  project: WikiProject,
  sourcePaths: string[],
  llmConfig: LlmConfig,
): Promise<string[]> {
  const pp = normalizePath(project.path)
  const importedPaths: string[] = []

  for (const sourcePath of sourcePaths) {
    const originalName = getFileName(sourcePath) || "unknown"
    const destPath = await getUniqueDestPath(`${pp}/raw/sources`, originalName)
    try {
      await copyFile(sourcePath, destPath)
      importedPaths.push(destPath)
      preprocessFile(destPath).catch(() => {})
    } catch (err) {
      console.error(`Failed to import ${originalName}:`, err)
    }
  }

  if (hasUsableLlm(llmConfig)) {
    for (const destPath of importedPaths) {
      enqueueIngest(project.id, destPath).catch((err) =>
        console.error("Failed to enqueue ingest:", err),
      )
    }
  }

  return importedPaths
}

export async function importSourceFolder(
  project: WikiProject,
  selectedFolder: string,
  llmConfig: LlmConfig,
): Promise<string[]> {
  const pp = normalizePath(project.path)
  const folderName = getFileName(selectedFolder) || "imported"
  const destDir = `${pp}/raw/sources/${folderName}`
  const copiedFiles: string[] = await invoke("copy_directory", {
    source: selectedFolder,
    destination: destDir,
  })

  for (const filePath of copiedFiles) {
    preprocessFile(filePath).catch(() => {})
  }

  if (hasUsableLlm(llmConfig)) {
    await enqueueSourceIngest(project, copiedFiles, llmConfig, {
      sourceRoot: destDir,
      rootContext: folderName,
    })
  }

  return copiedFiles
}

export async function deleteSourceFile(
  projectPath: string,
  sourcePath: string,
  options: { fileAlreadyDeleted?: boolean; logReason?: string } = {},
): Promise<DeleteSourceResult> {
  const pp = normalizePath(projectPath)
  const normalizedSource = normalizePath(sourcePath)
  const fileName = normalizedSource.split("/").pop() ?? ""
  if (!fileName) return { deletedWikiPaths: [], rewrittenSourcePages: 0 }

  const relatedPages = await findRelatedWikiPages(pp, fileName)
  const candidatePages = new Set(relatedPages)

  try {
    const wikiTree = await listDirectory(`${pp}/wiki`)
    for (const file of flattenMd(wikiTree)) {
      try {
        const content = await readFile(file.path)
        if (parseSources(content).some((source) => sourceMatchesDeletedFile(source, fileName))) {
          candidatePages.add(file.path)
        }
      } catch {
        // Ignore unreadable pages; related-pages matching may still cover them.
      }
    }
  } catch (err) {
    console.warn("[source-lifecycle] failed to scan wiki sources during delete:", err)
  }

  if (!options.fileAlreadyDeleted) {
    await deleteFile(normalizedSource)
  }

  try {
    await deleteFile(`${pp}/raw/sources/.cache/${fileName}.txt`)
  } catch {
    // cache file may not exist
  }

  const pagesToDelete: string[] = []
  let rewrittenSourcePages = 0

  for (const pagePath of candidatePages) {
    try {
      const content = await readFile(pagePath)
      const decision = decidePageFate(parseSources(content), fileName)
      if (decision.action === "keep") {
        await writeFile(pagePath, writeSources(content, decision.updatedSources))
        rewrittenSourcePages++
      } else if (decision.action === "delete") {
        pagesToDelete.push(pagePath)
      }
    } catch (err) {
      console.error(`Failed to process wiki page ${pagePath}:`, err)
    }
  }

  let deletedWikiPaths: string[] = []
  if (pagesToDelete.length > 0) {
    const { cascadeDeleteWikiPagesWithRefs } = await import("@/lib/wiki-page-delete")
    const result = await cascadeDeleteWikiPagesWithRefs(pp, pagesToDelete)
    deletedWikiPaths = result.deletedPaths
  }

  try {
    await removeFromIngestCache(pp, fileName)
  } catch {
    // non-critical
  }

  await appendSourceDeleteLog(pp, fileName, {
    reason: options.logReason ?? (options.fileAlreadyDeleted ? "external delete" : "delete"),
    deletedWikiCount: deletedWikiPaths.length,
    keptWikiCount: rewrittenSourcePages,
  })

  return { deletedWikiPaths, rewrittenSourcePages }
}

export async function deleteSourceFolder(
  projectPath: string,
  folder: FileNode,
  options: { folderAlreadyDeleted?: boolean } = {},
): Promise<DeleteSourceFolderResult> {
  const deletedWikiPaths: string[] = []
  for (const file of collectAllFilesIncludingDot(folder)) {
    try {
      const result = await deleteSourceFile(projectPath, file.path, {
        fileAlreadyDeleted: options.folderAlreadyDeleted,
        logReason: options.folderAlreadyDeleted ? "external folder delete" : "folder delete",
      })
      deletedWikiPaths.push(...result.deletedWikiPaths)
    } catch (err) {
      console.warn(`Failed to delete ${file.path} during source folder delete:`, err)
    }
  }

  if (!options.folderAlreadyDeleted) {
    try {
      await deleteFile(folder.path)
    } catch (err) {
      console.warn(`Failed to remove folder ${folder.path}:`, err)
    }
  }

  return { deletedWikiPaths }
}

export async function cleanupDeletedWikiPages(
  projectPath: string,
  relativePaths: string[],
): Promise<void> {
  const pp = normalizePath(projectPath)
  const deletedInfos = relativePaths
    .map((path) => ({ slug: getFileStem(path), title: "" }))
    .filter((info) => info.slug.length > 0 && !info.slug.startsWith("."))

  if (deletedInfos.length === 0) return

  for (const info of deletedInfos) {
    await removePageEmbedding(pp, info.slug)
    try {
      await deleteFile(`${pp}/wiki/media/${info.slug}`)
    } catch {
      // only source-summary pages usually own media; absence is normal
    }
  }

  const deletedKeys = buildDeletedKeys(deletedInfos)
  const deletedSlugSet = new Set(deletedInfos.map((info) => normalizeCleanupKey(info.slug)))
  const wikiTree = await listDirectory(`${pp}/wiki`)
  const allMd = flattenMd(wikiTree)

  for (const file of allMd) {
    let content: string
    try {
      content = await readFile(file.path)
    } catch {
      continue
    }

    let updated = content
    if (file.path === `${pp}/wiki/index.md` || file.name === "index.md") {
      updated = cleanIndexListing(updated, deletedKeys)
    }
    updated = stripDeletedWikilinks(updated, deletedKeys)

    const related = parseFrontmatterArray(updated, "related")
    if (related.length > 0) {
      const filtered = related.filter((s) => !deletedSlugSet.has(normalizeCleanupKey(s)))
      if (filtered.length !== related.length) {
        updated = writeFrontmatterArray(updated, "related", filtered)
      }
    }

    if (updated !== content) {
      try {
        await writeFile(file.path, updated)
      } catch (err) {
        console.warn(`[source-lifecycle] failed to rewrite ${file.path}:`, err)
      }
    }
  }
}

async function getUniqueDestPath(dir: string, fileName: string): Promise<string> {
  const basePath = `${dir}/${fileName}`

  try {
    await readFile(basePath)
  } catch {
    return basePath
  }

  const ext = fileName.includes(".") ? fileName.slice(fileName.lastIndexOf(".")) : ""
  const nameWithoutExt = ext ? fileName.slice(0, -ext.length) : fileName
  const date = new Date().toISOString().slice(0, 10).replace(/-/g, "")

  const withDate = `${dir}/${nameWithoutExt}-${date}${ext}`
  try {
    await readFile(withDate)
  } catch {
    return withDate
  }

  for (let i = 2; i <= 99; i++) {
    const withCounter = `${dir}/${nameWithoutExt}-${date}-${i}${ext}`
    try {
      await readFile(withCounter)
    } catch {
      return withCounter
    }
  }

  return `${dir}/${nameWithoutExt}-${date}-${Date.now()}${ext}`
}

async function appendSourceDeleteLog(
  projectPath: string,
  fileName: string,
  detail: { reason: string; deletedWikiCount: number; keptWikiCount: number },
): Promise<void> {
  try {
    const logPath = `${projectPath}/wiki/log.md`
    const logContent = await readFile(logPath).catch(() => "# Wiki Log\n")
    const date = new Date().toISOString().slice(0, 10)
    const logEntry = `\n## [${date}] ${detail.reason} | ${fileName}\n\nDeleted source file and ${detail.deletedWikiCount} wiki pages.${detail.keptWikiCount > 0 ? ` ${detail.keptWikiCount} shared pages kept (have other sources).` : ""}\n`
    await writeFile(logPath, logContent.trimEnd() + logEntry)
  } catch {
    // non-critical
  }
}

function flattenMd(nodes: readonly FileNode[]): FileNode[] {
  const out: FileNode[] = []
  function walk(items: readonly FileNode[]): void {
    for (const item of items) {
      if (item.is_dir) {
        if (item.children) walk(item.children)
      } else if (item.name.endsWith(".md")) {
        out.push(item)
      }
    }
  }
  walk(nodes)
  return out
}

function sourceMatchesDeletedFile(source: string, fileName: string): boolean {
  const normalizedSource = normalizePath(source).split("/").pop()?.toLowerCase() ?? ""
  return normalizedSource === fileName.toLowerCase()
}

function normalizeCleanupKey(value: string): string {
  return value.toLowerCase().replace(/[\s\-_]+/g, "")
}

function withRootContext(context: string, rootContext?: string): string {
  if (!rootContext) return context
  if (!context) return rootContext
  return `${rootContext} > ${context}`
}
