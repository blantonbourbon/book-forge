import { describe, expect, it } from "vitest";

import type { ApiErrorBody, JobResponse } from "./api";
import {
  READER_GUIDANCE_COPY,
  downloadGateForJob,
  jobPresentationForJob,
  pollingFailurePresentation,
  preStartFailurePresentation,
  refreshRecoveryPresentation,
  statusMessageDisplayText,
} from "./terminal-state";

describe("terminal job presentation", () => {
  it("labels completed jobs with warnings while keeping the current EPUB downloadable", () => {
    const job = jobResponse({
      id: "job-with-warnings",
      status: "completed",
      warnings: [
        {
          code: "image_fetch_failed",
          message: "Image was skipped because it could not be fetched.",
          affected: "https://example.test/missing.png",
        },
      ],
      downloadUrl: "/api/jobs/job-with-warnings/download",
    });

    const presentation = jobPresentationForJob(job, "http://127.0.0.1:3100");

    expect(presentation.phase).toBe("completed");
    expect(presentation.statusText).toContain("Completed with warnings");
    expect(presentation.warningHeading).toBe("Warnings (EPUB is ready)");
    expect(presentation.download).toMatchObject({
      available: true,
      href: "http://127.0.0.1:3100/api/jobs/job-with-warnings/download",
    });
    expect(presentation.canStartAnother).toBe(true);
    expect(statusMessageDisplayText(job.warnings[0])).toBe(
      "image_fetch_failed: Image was skipped because it could not be fetched. (https://example.test/missing.png)",
    );
  });

  it("never carries a stale completed download into running or failed jobs", () => {
    const completed = jobResponse({
      id: "completed-job",
      status: "completed",
      downloadUrl: "/api/jobs/completed-job/download",
    });
    const running = jobResponse({
      id: "new-running-job",
      status: "running",
      downloadUrl: "/api/jobs/completed-job/download",
    });
    const failed = jobResponse({
      id: "failed-job",
      status: "failed",
      errors: [{ code: "fetch_failed", message: "Source content failed." }],
      downloadUrl: "/api/jobs/completed-job/download",
    });

    expect(downloadGateForJob(completed, "").href).toBe(
      "/api/jobs/completed-job/download",
    );
    expect(downloadGateForJob(running, "")).toMatchObject({
      available: false,
      href: null,
    });
    expect(jobPresentationForJob(failed)).toMatchObject({
      phase: "failed",
      canStartAnother: true,
      download: {
        available: false,
        href: null,
      },
    });
  });
});

describe("submission and refresh recovery presentation", () => {
  it("keeps pre-start failures retryable without progress or download affordances", () => {
    const error: ApiErrorBody = {
      code: "validation_failed",
      message: "The source URL was rejected before a job was accepted.",
      fields: ["sourceUrl"],
    };

    const presentation = preStartFailurePresentation(error);

    expect(presentation.phase).toBe("failed");
    expect(presentation.statusText).toContain("Job was not started");
    expect(presentation.errors).toEqual([error]);
    expect(presentation).toMatchObject({ progressVisible: false });
    expect(presentation.canEditForm).toBe(true);
    expect(presentation.canSubmit).toBe(true);
    expect(presentation.download.available).toBe(false);
    expect(presentation.download.href).toBeNull();
  });

  it("treats polling refresh failures as recoverable failed states", () => {
    const error: ApiErrorBody = {
      code: "network_error",
      message: "The server could not be reached.",
      fields: [],
    };

    const presentation = pollingFailurePresentation(error);

    expect(presentation.phase).toBe("failed");
    expect(presentation.statusText).toContain(
      "Progress could not be refreshed",
    );
    expect(presentation.canSubmit).toBe(true);
    expect(presentation.download.available).toBe(false);
  });

  it("starts clean after browser refresh without duplicating a prior job", () => {
    const presentation = refreshRecoveryPresentation();

    expect(presentation.phase).toBe("idle");
    expect(presentation.currentJobId).toBeNull();
    expect(presentation.download.available).toBe(false);
    expect(presentation.canSubmit).toBe(true);
    expect(presentation.statusText).toContain("Configure a source URL");
  });
});

describe("reader guidance copy", () => {
  it("mentions WeRead and uses the canonical download label", () => {
    expect(READER_GUIDANCE_COPY.body).toContain("WeRead");
    expect(READER_GUIDANCE_COPY.downloadLabel).toBe("Download EPUB");
  });
});

function jobResponse(overrides: Partial<JobResponse>): JobResponse {
  return {
    id: "job-id",
    status: "queued",
    mode: "single",
    summary: {},
    progress: { percent: 0 },
    warnings: [],
    errors: [],
    ...overrides,
  };
}
