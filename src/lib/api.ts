//! HTTP transport for the LLM Wiki server.
//!
//! Used by every frontend file that previously called `invoke('foo', ...)`.
//! Each call site becomes `apiCall('METHOD', '/api/v1/foo', body?)`.

export interface ApiErrorBody {
  code: string;
  message: string;
  details?: unknown;
}

/** Typed error thrown by `apiCall` for any non-2xx response. */
export class ApiError extends Error {
  constructor(
    public readonly status: number,
    public readonly code: string,
    message: string,
    public readonly details?: unknown,
  ) {
    super(message);
    this.name = "ApiError";
  }

  static unauthenticated(): ApiError {
    return new ApiError(401, "UNAUTHENTICATED", "authentication required");
  }

  /** True if this represents a 401 — frontends use this to show the login view. */
  get isUnauthenticated(): boolean {
    return this.status === 401;
  }
}

// Default base URL: same origin (production / dev with Vite proxy).
// Tests can override via the `VITE_API_BASE` env var.
const BASE_URL = (import.meta.env?.VITE_API_BASE as string | undefined) ?? "";

export interface ApiCallOptions {
  /** Extra headers to set on the request. */
  headers?: Record<string, string>;
  /** AbortSignal for cancellation. */
  signal?: AbortSignal;
}

export async function apiCall<TRes = unknown>(
  method: "GET" | "POST" | "PUT" | "DELETE",
  path: string,
  body?: unknown,
  options: ApiCallOptions = {},
): Promise<TRes> {
  const url = `${BASE_URL}${path}`;
  const headers: Record<string, string> = { ...(options.headers ?? {}) };
  let bodyInit: BodyInit | undefined;
  if (body !== undefined) {
    headers["Content-Type"] = "application/json";
    bodyInit = JSON.stringify(body);
  }
  const resp = await fetch(url, {
    method,
    credentials: "include",
    headers,
    body: bodyInit,
    signal: options.signal,
  });
  if (!resp.ok) {
    let parsed: { error?: ApiErrorBody } | undefined;
    try {
      const text = await resp.text();
      parsed = text ? (JSON.parse(text) as { error?: ApiErrorBody }) : undefined;
    } catch {
      // body wasn't JSON
    }
    const err = parsed?.error ?? { code: "UNKNOWN", message: resp.statusText };
    throw new ApiError(resp.status, err.code, err.message, err.details);
  }
  // 204 / empty body
  if (resp.status === 204) {
    return undefined as unknown as TRes;
  }
  const ct = resp.headers.get("content-type") ?? "";
  if (ct.includes("application/json")) {
    return (await resp.json()) as TRes;
  }
  return (await resp.text()) as unknown as TRes;
}

/** Returns a raw `Response` (no JSON parsing). Use for binary content / streams. */
export async function apiFetch(
  method: "GET" | "POST" | "PUT" | "DELETE",
  path: string,
  body?: unknown,
  options: ApiCallOptions = {},
): Promise<Response> {
  const url = `${BASE_URL}${path}`;
  const headers: Record<string, string> = { ...(options.headers ?? {}) };
  let bodyInit: BodyInit | undefined;
  if (body !== undefined) {
    headers["Content-Type"] = "application/json";
    bodyInit = JSON.stringify(body);
  }
  return await fetch(url, {
    method,
    credentials: "include",
    headers,
    body: bodyInit,
    signal: options.signal,
  });
}

/** Builds a URL to the file preview endpoint with proper escaping. */
export function fileRawUrl(projectPath: string, filePath: string): string {
  const qs = new URLSearchParams({
    project_path: projectPath,
    path: filePath,
  });
  return `${BASE_URL}/api/v1/files/raw?${qs.toString()}`;
}
