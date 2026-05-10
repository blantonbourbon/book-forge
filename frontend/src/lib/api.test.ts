import { describe, expect, it } from "vitest";

import { createJob, downloadUrlForJob, getJob, resolveApiOrigin } from "./api";

describe("API origin resolution", () => {
  it("uses same-origin browser requests so dev proxy and production static serving work", () => {
    expect(resolveApiOrigin(new URL("http://127.0.0.1:3101/"))).toBe("");
    expect(resolveApiOrigin(new URL("http://127.0.0.1:3100/"))).toBe("");
    expect(resolveApiOrigin(new URL("https://book-forge.example/"))).toBe("");
  });
});

describe("Book Forge API client", () => {
  it("posts a canonical job creation payload", async () => {
    const payload = {
      sourceUrl: "https://example.test/single-page/index.html",
      mode: "single" as const,
      metadata: {
        title: "API Client",
        author: "Tester",
        language: "en",
        description: "",
      },
      options: {
        includeImages: true,
      },
    };
    const calls: Array<{ url: string; init: RequestInit | undefined }> = [];
    const fetcher = async (url: string, init?: RequestInit) => {
      calls.push({ url, init });
      return jsonResponse({
        id: "job-1",
        status: "queued",
        mode: "single",
        summary: payload,
        progress: { percent: 0 },
        warnings: [],
        errors: [],
      });
    };

    const response = await createJob(payload, {
      apiOrigin: "http://127.0.0.1:3100",
      fetcher,
    });

    expect(response.id).toBe("job-1");
    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({
      url: "http://127.0.0.1:3100/api/jobs",
      init: {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      },
    });
  });

  it("gets job status by id", async () => {
    const calls: Array<{ url: string; init: RequestInit | undefined }> = [];
    const fetcher = async (url: string, init?: RequestInit) => {
      calls.push({ url, init });
      return jsonResponse({
        id: "job-2",
        status: "completed",
        mode: "single",
        summary: {},
        progress: { percent: 100 },
        warnings: [],
        errors: [],
        downloadUrl: "/api/jobs/job-2/download",
      });
    };

    const response = await getJob("job-2", {
      apiOrigin: "",
      fetcher,
    });

    expect(response.status).toBe("completed");
    expect(calls).toEqual([{ url: "/api/jobs/job-2", init: undefined }]);
  });

  it("throws safe API errors from JSON error envelopes", async () => {
    const fetcher = async () =>
      jsonResponse(
        {
          error: {
            code: "validation_failed",
            message: "One or more job request fields were invalid.",
            fields: ["sourceUrl"],
          },
        },
        422,
      );

    await expect(
      createJob(
        {
          sourceUrl: "file:///etc/passwd",
          mode: "single",
          metadata: {
            title: "Bad",
            author: "Tester",
            language: "en",
            description: "",
          },
          options: { includeImages: false },
        },
        { apiOrigin: "", fetcher },
      ),
    ).rejects.toMatchObject({
      code: "validation_failed",
      message: "One or more job request fields were invalid.",
      fields: ["sourceUrl"],
      status: 422,
    });
  });

  it("gates downloads to completed jobs with a current download URL", () => {
    const base = {
      id: "job-1",
      mode: "single" as const,
      summary: {},
      progress: { percent: 0 },
      warnings: [],
      errors: [],
    };

    expect(
      downloadUrlForJob(
        {
          ...base,
          status: "completed" as const,
          downloadUrl: "/api/jobs/job-1/download",
        },
        "http://127.0.0.1:3100",
      ),
    ).toBe("http://127.0.0.1:3100/api/jobs/job-1/download");
    expect(
      downloadUrlForJob({
        ...base,
        status: "completed" as const,
      }),
    ).toBeNull();

    for (const status of ["queued", "running", "failed"] as const) {
      expect(
        downloadUrlForJob({
          ...base,
          status,
          downloadUrl: "/api/jobs/job-1/download",
        }),
      ).toBeNull();
    }
  });
});

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "Content-Type": "application/json",
    },
  });
}
