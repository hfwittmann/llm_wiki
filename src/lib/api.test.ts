import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { apiCall, ApiError, fileRawUrl } from "./api";

// Helper to mock fetch with a specific response.
function mockFetch(opts: {
  status: number;
  body?: unknown;
  contentType?: string;
}) {
  const ct = opts.contentType ?? "application/json";
  const bodyStr =
    typeof opts.body === "string"
      ? opts.body
      : opts.body !== undefined
        ? JSON.stringify(opts.body)
        : "";
  const fetchMock = vi.fn().mockResolvedValue(
    new Response(bodyStr, {
      status: opts.status,
      headers: { "content-type": ct },
    }),
  );
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

describe("apiCall", () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("sends a GET without body", async () => {
    const fetchMock = mockFetch({ status: 200, body: { ok: true } });
    const result = await apiCall<{ ok: boolean }>("GET", "/api/v1/health");
    expect(result.ok).toBe(true);
    const [, init] = fetchMock.mock.calls[0];
    expect(init.method).toBe("GET");
    expect(init.body).toBeUndefined();
    expect(init.credentials).toBe("include");
  });

  it("sends a POST with JSON body and content-type header", async () => {
    const fetchMock = mockFetch({ status: 200, body: {} });
    await apiCall("POST", "/api/v1/foo", { x: 1 });
    const [, init] = fetchMock.mock.calls[0];
    expect(init.method).toBe("POST");
    expect(init.headers["Content-Type"]).toBe("application/json");
    expect(init.body).toBe(JSON.stringify({ x: 1 }));
  });

  it("returns undefined for 204 No Content", async () => {
    // Response constructor rejects 204 with a body per the Fetch spec;
    // stub fetch directly with a null-body 204 response.
    const fetchMock = vi.fn().mockResolvedValue(new Response(null, { status: 204 }));
    vi.stubGlobal("fetch", fetchMock);
    const result = await apiCall("POST", "/api/v1/auth/logout");
    expect(result).toBeUndefined();
  });

  it("throws ApiError with code/message on 401", async () => {
    mockFetch({
      status: 401,
      body: {
        error: {
          code: "UNAUTHENTICATED",
          message: "authentication required",
        },
      },
    });
    try {
      await apiCall("GET", "/api/v1/auth/whoami");
      expect.fail("should have thrown");
    } catch (e) {
      expect(e).toBeInstanceOf(ApiError);
      const err = e as ApiError;
      expect(err.status).toBe(401);
      expect(err.code).toBe("UNAUTHENTICATED");
      expect(err.isUnauthenticated).toBe(true);
    }
  });

  it("throws ApiError with UNKNOWN code on non-JSON error body", async () => {
    mockFetch({
      status: 500,
      body: "internal",
      contentType: "text/plain",
    });
    try {
      await apiCall("GET", "/api/v1/health");
      expect.fail("should have thrown");
    } catch (e) {
      const err = e as ApiError;
      expect(err.status).toBe(500);
      expect(err.code).toBe("UNKNOWN");
    }
  });

  it("returns text body for non-JSON content-type", async () => {
    mockFetch({
      status: 200,
      body: "# Markdown",
      contentType: "text/markdown",
    });
    const result = await apiCall<string>("GET", "/api/v1/files/raw");
    expect(result).toBe("# Markdown");
  });
});

describe("fileRawUrl", () => {
  it("escapes project and file paths", () => {
    const url = fileRawUrl("research/thesis", "wiki/index.md");
    expect(url).toContain("project_path=research%2Fthesis");
    expect(url).toContain("path=wiki%2Findex.md");
  });
});
