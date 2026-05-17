import { downloadUrlForJob, type ApiErrorBody, type JobResponse } from "./api";
import { canSubmitInPhase, type UiPhase } from "./form-state";

export interface DownloadGate {
  available: boolean;
  href: string | null;
  label: string;
  unavailableReason: string;
}

export interface JobPresentation {
  phase: UiPhase;
  statusText: string;
  progressVisible: boolean;
  warningHeading: string;
  errorHeading: string;
  canStartAnother: boolean;
  canSubmit: boolean;
  download: DownloadGate;
}

export interface FailurePresentation {
  phase: "failed";
  statusText: string;
  progressVisible: boolean;
  errors: ApiErrorBody[];
  canEditForm: boolean;
  canSubmit: boolean;
  download: DownloadGate;
}

export interface RefreshRecoveryPresentation {
  phase: "idle";
  statusText: string;
  currentJobId: null;
  canSubmit: boolean;
  download: DownloadGate;
}

export const DOWNLOAD_LABEL = "Download EPUB";

export const READER_GUIDANCE_COPY = {
  downloadLabel: DOWNLOAD_LABEL,
  body: "Import the EPUB into WeRead or any reader after download.",
} as const;

export function downloadGateForJob(
  job: JobResponse | null,
  apiOrigin?: string,
): DownloadGate {
  const href = job ? downloadUrlForJob(job, apiOrigin) : null;
  return {
    available: href !== null,
    href,
    label: DOWNLOAD_LABEL,
    unavailableReason:
      "An EPUB download is available only for the current completed job.",
  };
}

export function statusMessageDisplayText(message: {
  code: string;
  message: string;
  affected?: string;
}): string {
  const prefix = message.code ? `${message.code}: ` : "";
  return message.affected
    ? `${prefix}${message.message} (${message.affected})`
    : `${prefix}${message.message}`;
}

export function jobPresentationForJob(
  job: JobResponse,
  apiOrigin?: string,
): JobPresentation {
  const phase = phaseForJobStatus(job.status);
  const download = downloadGateForJob(job, apiOrigin);
  return {
    phase,
    statusText: statusTextForJob(job),
    progressVisible: job.status !== "failed",
    warningHeading:
      job.status === "completed" && job.warnings.length
        ? "Warnings (EPUB is ready)"
        : "Warnings",
    errorHeading: "Errors",
    canStartAnother: phase === "completed" || phase === "failed",
    canSubmit: canSubmitInPhase(phase),
    download,
  };
}

export function preStartFailurePresentation(
  error: ApiErrorBody,
): FailurePresentation {
  return {
    phase: "failed",
    statusText: `Job was not started: ${error.message}`,
    progressVisible: false,
    errors: [error],
    canEditForm: true,
    canSubmit: true,
    download: emptyDownloadGate(),
  };
}

export function pollingFailurePresentation(
  error: ApiErrorBody,
): FailurePresentation {
  return {
    phase: "failed",
    statusText: `Progress could not be refreshed: ${error.message}`,
    progressVisible: false,
    errors: [error],
    canEditForm: true,
    canSubmit: true,
    download: emptyDownloadGate(),
  };
}

export function refreshRecoveryPresentation(): RefreshRecoveryPresentation {
  return {
    phase: "idle",
    statusText: "Configure a source URL to begin.",
    currentJobId: null,
    canSubmit: true,
    download: emptyDownloadGate(),
  };
}

function phaseForJobStatus(status: JobResponse["status"]): UiPhase {
  if (status === "queued" || status === "running") {
    return "polling";
  }
  return status;
}

function statusTextForJob(job: JobResponse): string {
  const label = statusLabelForJob(job);
  const percent = clampProgress(job.progress?.percent ?? 0);
  const details = [
    `${percent}%`,
    job.progress?.pagesFetched !== undefined
      ? `${job.progress.pagesFetched} fetched`
      : undefined,
    job.progress?.pagesSkipped !== undefined
      ? `${job.progress.pagesSkipped} skipped`
      : undefined,
  ].filter(Boolean);

  return `${label}: ${details.join(", ")}.`;
}

function statusLabelForJob(job: JobResponse): string {
  if (job.status === "completed" && job.warnings.length) {
    return "Completed with warnings";
  }

  return {
    queued: "Queued",
    running: "Running",
    completed: "Completed",
    failed: "Failed",
  }[job.status];
}

function emptyDownloadGate(): DownloadGate {
  return {
    available: false,
    href: null,
    label: DOWNLOAD_LABEL,
    unavailableReason:
      "An EPUB download is available only for the current completed job.",
  };
}

function clampProgress(value: number): number {
  if (!Number.isFinite(value)) {
    return 0;
  }
  return Math.max(0, Math.min(100, Math.round(value)));
}
