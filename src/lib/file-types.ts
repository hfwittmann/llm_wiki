export type FileCategory =
  | "markdown"
  | "text"
  | "code"
  | "image"
  | "video"
  | "audio"
  | "pdf"
  | "document"
  | "data"
  | "unknown"

const EXT_MAP: Record<string, FileCategory> = {
  // Markdown
  md: "markdown",
  mdx: "markdown",

  // Text
  txt: "text",
  rtf: "text",
  log: "text",

  // Code
  js: "code",
  jsx: "code",
  ts: "code",
  tsx: "code",
  py: "code",
  rs: "code",
  go: "code",
  java: "code",
  c: "code",
  cpp: "code",
  h: "code",
  hpp: "code",
  rb: "code",
  php: "code",
  swift: "code",
  kt: "code",
  scala: "code",
  sh: "code",
  bash: "code",
  zsh: "code",
  sql: "code",
  r: "code",
  lua: "code",
  css: "code",
  scss: "code",
  less: "code",
  html: "code",
  htm: "code",
  xml: "code",
  svg: "code",
  vue: "code",
  svelte: "code",
  toml: "code",
  ini: "code",
  cfg: "code",
  conf: "code",
  dockerfile: "code",
  makefile: "code",

  // Images
  png: "image",
  jpg: "image",
  jpeg: "image",
  gif: "image",
  webp: "image",
  bmp: "image",
  ico: "image",
  tiff: "image",
  tif: "image",
  avif: "image",
  heic: "image",
  heif: "image",

  // Video
  mp4: "video",
  webm: "video",
  mov: "video",
  avi: "video",
  mkv: "video",
  flv: "video",
  wmv: "video",
  m4v: "video",

  // Audio
  mp3: "audio",
  wav: "audio",
  ogg: "audio",
  flac: "audio",
  aac: "audio",
  m4a: "audio",
  wma: "audio",

  // PDF
  pdf: "pdf",

  // Documents. Some are previewable through backend text extraction.
  doc: "document",
  docx: "document",
  xls: "document",
  xlsx: "document",
  ppt: "document",
  pptx: "document",
  odt: "document",
  ods: "document",
  odp: "document",
  pages: "document",
  numbers: "document",
  key: "document",
  epub: "document",

  // Data
  json: "data",
  jsonl: "data",
  csv: "data",
  tsv: "data",
  yaml: "data",
  yml: "data",
  ndjson: "data",
}

export function getFileCategory(filePath: string): FileCategory {
  const ext = filePath.split(".").pop()?.toLowerCase() ?? ""
  return EXT_MAP[ext] ?? "unknown"
}

export function isTextReadable(category: FileCategory): boolean {
  return ["markdown", "text", "code", "data"].includes(category)
}

export const EXTRACTED_TEXT_PREVIEW_EXTENSIONS = new Set([
  "pdf",
  "doc",
  "docx",
  "pptx",
  "xls",
  "xlsx",
  "odt",
  "ods",
  "odp",
])

export function getFileExtension(filePath: string): string {
  const fileName = filePath.split(/[\\/]/).pop() ?? ""
  return fileName.includes(".") ? fileName.split(".").pop()?.toLowerCase() ?? "" : ""
}

export function isExtractedTextPreviewFile(_filePath: string): boolean {
  // Browser/LAN-port disabled: the legacy Tauri path used pdfium-extracted
  // text as a text-only preview for PDF/DOCX/etc. The HTTP /files/raw
  // endpoint streams raw bytes (no server-side text extraction), so loading
  // a 1+ MB binary as a string locks up the browser. Returning false here
  // makes the preview-panel hit the binary short-circuit and skip readFile
  // for these types; binary preview UI (PDF embed via fileRawUrl) handles
  // rendering. Once a /files/extracted-text endpoint exists, this can be
  // restored to the original extension-set lookup.
  return false
}

export function isBinary(category: FileCategory): boolean {
  return ["image", "video", "audio", "document", "unknown"].includes(category)
}

export function getCodeLanguage(filePath: string): string {
  const ext = filePath.split(".").pop()?.toLowerCase() ?? ""
  const langMap: Record<string, string> = {
    js: "javascript",
    jsx: "javascript",
    ts: "typescript",
    tsx: "typescript",
    py: "python",
    rs: "rust",
    go: "go",
    java: "java",
    rb: "ruby",
    php: "php",
    swift: "swift",
    sql: "sql",
    html: "html",
    htm: "html",
    css: "css",
    json: "json",
    yaml: "yaml",
    yml: "yaml",
    xml: "xml",
    sh: "bash",
    bash: "bash",
    toml: "toml",
  }
  return langMap[ext] ?? ext
}
