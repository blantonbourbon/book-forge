import { DEFAULT_API_ORIGIN } from "./config";
import type { CreateJobPayload, ConversionMode } from "./form-state";

export type JobLifecycleStatus = "queued" | "running" | "completed" | "failed";

export interface ApiWarning {
  code: string;
  message: string;
  affected?: string;
}

export interface ApiErrorBody {
  code: string;
  message: string;
  fields?: string[];
}

export interface JobProgress {
  percent: number;
  pagesDiscovered?: number;
  pagesFetched?: number;
  pagesSkipped?: number;
  currentDepth?: number;
  bytesFetched?: number;
  maxPages?: number;
  maxDepth?: number;
  maxTotalBytes?: number;
}

export interface JobResponse {
  id: string;
  status: JobLifecycleStatus;
  mode: ConversionMode;
  summary: unknown;
  progress: JobProgress;
  warnings: ApiWarning[];
  errors: ApiErrorBody[];
  downloadUrl?: string;
}

export interface MetadataPreview {
  title: string;
  author: string;
  description: string;
  finalUrl: string;
}

export interface AuthUser {
  id: number;
  login: string;
  name?: string;
  avatarUrl?: string;
  email?: string;
}

export interface AuthSession {
  authRequired: boolean;
  authenticated: boolean;
  user?: AuthUser;
  loginUrl?: string;
  logoutUrl?: string;
}

export interface ApiClientOptions {
  apiOrigin?: string;
  fetcher?: FetchLike;
}

export interface MetadataPreviewOptions extends ApiClientOptions {
  useBrowser?: boolean;
}

export type FetchLike = (url: string, init?: RequestInit) => Promise<Response>;

export class ApiClientError extends Error {
  readonly status: number;
  readonly code: string;
  readonly fields: string[];

  constructor(status: number, error: ApiErrorBody) {
    super(error.message);
    this.name = "ApiClientError";
    this.status = status;
    this.code = error.code;
    this.fields = error.fields ?? [];
  }
}

export function resolveApiOrigin(
  currentLocation: URL | Location | undefined = getBrowserLocation(),
): string {
  if (!currentLocation) {
    return DEFAULT_API_ORIGIN;
  }

  return "";
}

export async function createJob(
  payload: CreateJobPayload,
  options: ApiClientOptions = {},
): Promise<JobResponse> {
  return requestJson<JobResponse>("/api/jobs", options, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export async function getJob(
  id: string,
  options: ApiClientOptions = {},
): Promise<JobResponse> {
  return requestJson<JobResponse>(
    `/api/jobs/${encodeURIComponent(id)}`,
    options,
  );
}

export async function previewMetadata(
  sourceUrl: string,
  options: MetadataPreviewOptions = {},
): Promise<MetadataPreview> {
  const params = new URLSearchParams({ url: sourceUrl });
  if (options.useBrowser) {
    params.set("useBrowser", "true");
  }
  return requestJson<MetadataPreview>(`/api/preview?${params}`, options);
}

export async function getAuthSession(
  options: ApiClientOptions = {},
): Promise<AuthSession> {
  return requestJson<AuthSession>("/api/auth/session", options);
}

export async function signOut(
  options: ApiClientOptions = {},
): Promise<AuthSession> {
  return requestJson<AuthSession>("/api/auth/logout", options, {
    method: "POST",
  });
}

export function loginUrlForReturnTo(
  returnTo = getBrowserReturnTo(),
  apiOrigin = resolveApiOrigin(),
): string {
  const params = new URLSearchParams({ returnTo });
  return apiUrl(`/api/auth/login?${params}`, apiOrigin);
}

export function downloadUrlForJob(
  job: JobResponse,
  apiOrigin = resolveApiOrigin(),
): string | null {
  if (job.status !== "completed" || !job.downloadUrl) {
    return null;
  }
  return apiUrl(job.downloadUrl, apiOrigin);
}

async function requestJson<T>(
  path: string,
  options: ApiClientOptions,
  init?: RequestInit,
): Promise<T> {
  const fetcher = options.fetcher ?? fetch.bind(globalThis);
  const response = await fetcher(
    apiUrl(path, options.apiOrigin ?? resolveApiOrigin()),
    init,
  );
  const body = await parseJson(response);

  if (!response.ok) {
    throw new ApiClientError(response.status, errorFromBody(body));
  }

  return body as T;
}

async function parseJson(response: Response): Promise<unknown> {
  try {
    return await response.json();
  } catch {
    if (!response.ok) {
      return {
        error: {
          code: "http_error",
          message: `Request failed with status ${response.status}.`,
        },
      };
    }
    throw new ApiClientError(response.status, {
      code: "invalid_api_response",
      message: "The server returned an unreadable response.",
    });
  }
}

function errorFromBody(body: unknown): ApiErrorBody {
  if (
    typeof body === "object" &&
    body !== null &&
    "error" in body &&
    typeof body.error === "object" &&
    body.error !== null
  ) {
    const error = body.error as Partial<ApiErrorBody>;
    return {
      code: safeString(error.code, "request_failed"),
      message: safeString(error.message, "The request could not be completed."),
      fields: Array.isArray(error.fields)
        ? error.fields.filter(
            (field): field is string => typeof field === "string",
          )
        : [],
    };
  }

  return {
    code: "request_failed",
    message: "The request could not be completed.",
    fields: [],
  };
}

function safeString(value: unknown, fallback: string): string {
  return typeof value === "string" && value.trim() ? value : fallback;
}

function apiUrl(path: string, apiOrigin: string): string {
  if (!apiOrigin) {
    return path;
  }
  return `${apiOrigin.replace(/\/$/, "")}${path}`;
}

function getBrowserLocation(): Location | undefined {
  return typeof globalThis.location === "undefined"
    ? undefined
    : globalThis.location;
}

function getBrowserReturnTo(): string {
  const location = getBrowserLocation();
  if (!location) {
    return "/";
  }
  return `${location.pathname}${location.search}${location.hash}`;
}
