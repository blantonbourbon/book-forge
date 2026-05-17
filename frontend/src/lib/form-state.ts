export type ConversionMode = "single" | "crawl";
export type ChapterStrategy = "source-order" | "single-document";
export type OutputTarget = "epub";
export type UiPhase =
  | "idle"
  | "submitting"
  | "polling"
  | "completed"
  | "failed";

export interface FormValues {
  sourceUrl: string;
  mode: ConversionMode;
  title: string;
  author: string;
  description: string;
  includeImages: boolean;
  useBrowser: boolean;
  chapterStrategy: ChapterStrategy;
  outputTarget: OutputTarget;
  crawlPrefixUrl: string;
  maxDepth: string;
  maxPages: string;
}

export interface CreateJobPayload {
  sourceUrl: string;
  mode: ConversionMode;
  metadata: {
    title: string;
    author: string;
    description: string;
  };
  options: {
    includeImages: boolean;
    outputTarget: OutputTarget;
    useBrowser: boolean;
  };
  crawl?: {
    prefixUrl: string;
    maxDepth: number;
    maxPages: number;
    maxTotalBytes: number;
    maxDurationMillis: number;
  };
}

export type FormField = keyof FormValues;
export type ValidationErrors = Partial<Record<FormField, string[]>>;

export interface ValidationResult {
  canSubmit: boolean;
  errors: ValidationErrors;
  firstInvalidField?: FormField;
}

export const DEFAULT_MAX_TOTAL_BYTES = 10 * 1024 * 1024;
export const DEFAULT_MAX_DURATION_MILLIS = 30_000;
export const MAX_CRAWL_DEPTH = 10;
export const MAX_CRAWL_PAGES = 100;

export const DEFAULT_FORM_VALUES: FormValues = {
  sourceUrl: "",
  mode: "single",
  title: "Untitled Book",
  author: "Unknown Author",
  description: "",
  includeImages: true,
  useBrowser: false,
  chapterStrategy: "source-order",
  outputTarget: "epub",
  crawlPrefixUrl: "",
  maxDepth: "3",
  maxPages: "50",
};

export function cloneDefaultFormValues(): FormValues {
  return { ...DEFAULT_FORM_VALUES };
}

export function applyMode(
  values: FormValues,
  mode: ConversionMode,
): FormValues {
  return {
    ...values,
    mode,
  };
}

export function canSubmitInPhase(phase: UiPhase): boolean {
  return !["submitting", "polling"].includes(phase);
}

export function validateForm(values: FormValues): ValidationResult {
  const errors: ValidationErrors = {};

  validateUrlField(
    values.sourceUrl,
    "sourceUrl",
    "Source URL",
    errors,
    "Source URL is required.",
  );

  if (!values.title.trim()) {
    addError(errors, "title", "Book title is required.");
  }

  if (values.mode === "crawl") {
    validateUrlField(
      values.crawlPrefixUrl,
      "crawlPrefixUrl",
      "Crawl prefix URL",
      errors,
      "Crawl prefix URL is required in crawl mode.",
    );
    validateIntegerField(
      values.maxDepth,
      "maxDepth",
      "Crawl depth",
      0,
      MAX_CRAWL_DEPTH,
      errors,
    );
    validateIntegerField(
      values.maxPages,
      "maxPages",
      "Page limit",
      1,
      MAX_CRAWL_PAGES,
      errors,
    );
    validateCrawlScope(values, errors);
  }

  const firstInvalidField = fieldOrder.find((field) => errors[field]?.length);

  return {
    canSubmit: firstInvalidField === undefined,
    errors,
    firstInvalidField,
  };
}

function validateCrawlScope(values: FormValues, errors: ValidationErrors) {
  if (errors.sourceUrl?.length || errors.crawlPrefixUrl?.length) {
    return;
  }

  const sourceUrl = new URL(values.sourceUrl.trim());
  const prefixUrl = new URL(values.crawlPrefixUrl.trim());
  if (sourceUrl.origin !== prefixUrl.origin) {
    addError(
      errors,
      "crawlPrefixUrl",
      "Crawl prefix must share the source URL origin.",
    );
    return;
  }

  const sourceWithoutFragment = stripFragment(sourceUrl).toString();
  const prefixWithoutFragment = stripFragment(prefixUrl).toString();
  if (!sourceWithoutFragment.startsWith(prefixWithoutFragment)) {
    addError(
      errors,
      "crawlPrefixUrl",
      "Crawl prefix must include the source URL path.",
    );
  }
}

export function buildCreateJobPayload(values: FormValues): CreateJobPayload {
  const payload: CreateJobPayload = {
    sourceUrl: values.sourceUrl.trim(),
    mode: values.mode,
    metadata: {
      title: values.title.trim(),
      author: values.author.trim(),
      description: values.description.trim(),
    },
    options: {
      includeImages: values.includeImages,
      outputTarget: values.outputTarget,
      useBrowser: values.useBrowser,
    },
  };

  if (values.mode === "crawl") {
    payload.crawl = {
      prefixUrl: values.crawlPrefixUrl.trim(),
      maxDepth: Number(values.maxDepth),
      maxPages: Number(values.maxPages),
      maxTotalBytes: DEFAULT_MAX_TOTAL_BYTES,
      maxDurationMillis: DEFAULT_MAX_DURATION_MILLIS,
    };
  }

  return payload;
}

export function outputOptionSummary(values: FormValues): string {
  const imageSummary = values.includeImages
    ? "Images will be embedded when safe."
    : "Images will be skipped without broken references.";
  const chapterSummary =
    values.chapterStrategy === "source-order"
      ? "Chapters follow source page order."
      : "Readable content is kept together where possible.";
  const browserNote = values.useBrowser ? " Browser rendering enabled." : "";
  return `${imageSummary} ${chapterSummary}${browserNote} Output is EPUB download only.`;
}

const fieldOrder: FormField[] = [
  "sourceUrl",
  "mode",
  "title",
  "author",
  "description",
  "includeImages",
  "useBrowser",
  "chapterStrategy",
  "outputTarget",
  "crawlPrefixUrl",
  "maxDepth",
  "maxPages",
];

function validateUrlField(
  rawValue: string,
  field: FormField,
  label: string,
  errors: ValidationErrors,
  requiredMessage: string,
) {
  const value = rawValue.trim();
  if (!value) {
    addError(errors, field, requiredMessage);
    return;
  }

  let parsed: URL;
  try {
    parsed = new URL(value);
  } catch {
    addError(errors, field, `Enter an absolute HTTP or HTTPS URL.`);
    return;
  }

  if (!["http:", "https:"].includes(parsed.protocol)) {
    addError(errors, field, "Only HTTP and HTTPS source URLs are supported.");
    return;
  }

  if (hostLooksUnsafe(parsed.hostname)) {
    addError(
      errors,
      field,
      `${label} cannot target localhost, private, link-local, or metadata addresses.`,
    );
  }
}

function validateIntegerField(
  rawValue: string,
  field: FormField,
  label: string,
  min: number,
  max: number,
  errors: ValidationErrors,
) {
  const value = rawValue.trim();
  if (!/^\d+$/.test(value)) {
    addError(
      errors,
      field,
      `${label} must be a whole number from ${min} to ${max}.`,
    );
    return;
  }

  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < min || parsed > max) {
    addError(errors, field, `${label} must be between ${min} and ${max}.`);
  }
}

function hostLooksUnsafe(hostname: string): boolean {
  const host = hostname
    .toLowerCase()
    .replace(/^\[/, "")
    .replace(/\]$/, "")
    .replace(/\.$/, "");
  if (
    host === "localhost" ||
    host === "localhost.localdomain" ||
    host.endsWith(".localhost")
  ) {
    return true;
  }

  const isIpv6Literal = host.includes(":");
  if (
    isIpv6Literal &&
    (host === "::1" ||
      host.startsWith("fe80:") ||
      host.startsWith("fc") ||
      host.startsWith("fd"))
  ) {
    return true;
  }

  const ipv4 = host.split(".").map((part) => Number(part));
  if (
    ipv4.length === 4 &&
    ipv4.every((part) => Number.isInteger(part) && part >= 0 && part <= 255)
  ) {
    const [first = 0, second = 0] = ipv4;
    return (
      first === 0 ||
      first === 10 ||
      first === 127 ||
      (first === 169 && second === 254) ||
      (first === 172 && second >= 16 && second <= 31) ||
      (first === 192 && second === 168)
    );
  }

  return false;
}

function stripFragment(url: URL): URL {
  const stripped = new URL(url);
  stripped.hash = "";
  return stripped;
}

function addError(errors: ValidationErrors, field: FormField, message: string) {
  errors[field] = [...(errors[field] ?? []), message];
}
