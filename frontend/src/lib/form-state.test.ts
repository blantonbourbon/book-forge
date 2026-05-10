import { describe, expect, it } from "vitest";

import {
  DEFAULT_FORM_VALUES,
  applyMode,
  buildCreateJobPayload,
  canSubmitInPhase,
  validateForm,
} from "./form-state";

describe("conversion form defaults and validation", () => {
  it("exposes deterministic first-load defaults", () => {
    expect(DEFAULT_FORM_VALUES).toEqual({
      sourceUrl: "",
      mode: "single",
      title: "Untitled Book",
      author: "Unknown Author",
      language: "en",
      description: "",
      includeImages: true,
      chapterStrategy: "source-order",
      outputTarget: "epub",
      crawlPrefixUrl: "",
      maxDepth: "3",
      maxPages: "50",
    });
  });

  it("rejects missing, malformed, unsupported, and invalid language input", () => {
    const base = { ...DEFAULT_FORM_VALUES };

    expect(validateForm({ ...base, sourceUrl: "" }).errors.sourceUrl).toContain(
      "Source URL is required.",
    );
    expect(validateForm({ ...base, title: "" }).errors.title).toContain(
      "Book title is required.",
    );
    expect(
      validateForm({ ...base, sourceUrl: "not a url" }).errors.sourceUrl,
    ).toContain("Enter an absolute HTTP or HTTPS URL.");
    expect(
      validateForm({ ...base, sourceUrl: "file:///tmp/book.html" }).errors
        .sourceUrl,
    ).toContain("Only HTTP and HTTPS source URLs are supported.");
    expect(
      validateForm({ ...base, sourceUrl: "http://127.0.0.1/private" }).errors
        .sourceUrl,
    ).toContain(
      "Source URL cannot target localhost, private, link-local, or metadata addresses.",
    );
    expect(
      validateForm({
        ...base,
        sourceUrl: "https://example.test/redirects/to-private",
      }).canSubmit,
    ).toBe(true);
    expect(
      validateForm({ ...base, language: "english!" }).errors.language,
    ).toContain("Use a valid language tag such as en or zh-CN.");
  });

  it("accepts HTTP and HTTPS source URLs when required fields are valid", () => {
    for (const sourceUrl of [
      "http://example.test/single-page/index.html",
      "https://example.test/single-page/index.html",
    ]) {
      const result = validateForm({ ...DEFAULT_FORM_VALUES, sourceUrl });

      expect(result.canSubmit).toBe(true);
      expect(result.errors.sourceUrl ?? []).toEqual([]);
    }
  });

  it("validates crawl prefix, depth, and page limits explicitly", () => {
    const crawl = {
      ...DEFAULT_FORM_VALUES,
      mode: "crawl" as const,
      sourceUrl: "https://example.test/crawl-graph/index.html",
      crawlPrefixUrl: "https://example.test/crawl-graph/",
    };

    expect(validateForm(crawl).canSubmit).toBe(true);
    expect(validateForm({ ...crawl, maxDepth: "0" }).canSubmit).toBe(true);

    for (const crawlPrefixUrl of ["", "not a url", "ftp://example.test/"]) {
      expect(
        validateForm({ ...crawl, crawlPrefixUrl }).errors.crawlPrefixUrl,
      ).toBeDefined();
    }

    expect(
      validateForm({
        ...crawl,
        crawlPrefixUrl: "https://other.example/crawl-graph/",
      }).errors.crawlPrefixUrl,
    ).toContain("Crawl prefix must share the source URL origin.");

    for (const maxDepth of ["-1", "1.5", "abc", "11"]) {
      expect(
        validateForm({ ...crawl, maxDepth }).errors.maxDepth,
      ).toBeDefined();
    }

    for (const maxPages of ["0", "-1", "2.5", "abc", "101"]) {
      expect(
        validateForm({ ...crawl, maxPages }).errors.maxPages,
      ).toBeDefined();
    }
  });
});

describe("conversion form payloads and state helpers", () => {
  it("preserves shared and crawl-specific values while switching modes", () => {
    const edited = {
      ...DEFAULT_FORM_VALUES,
      sourceUrl: "https://example.test/crawl-graph/index.html",
      title: "Edited Title",
      author: "Edited Author",
      description: "Edited description",
      includeImages: false,
      crawlPrefixUrl: "https://example.test/crawl-graph/",
      maxDepth: "2",
      maxPages: "9",
    };

    expect(applyMode(edited, "crawl")).toMatchObject({
      mode: "crawl",
      title: "Edited Title",
      includeImages: false,
      crawlPrefixUrl: "https://example.test/crawl-graph/",
      maxDepth: "2",
      maxPages: "9",
    });
    expect(applyMode({ ...edited, mode: "crawl" }, "single")).toMatchObject({
      mode: "single",
      title: "Edited Title",
      includeImages: false,
      crawlPrefixUrl: "https://example.test/crawl-graph/",
      maxDepth: "2",
      maxPages: "9",
    });
  });

  it("builds canonical single and crawl API payloads from preserved values", () => {
    const single = {
      ...DEFAULT_FORM_VALUES,
      sourceUrl: "https://example.test/single-page/index.html",
      title: "Single Book",
      author: "A. Reader",
      language: "en-US",
      description: "Single description",
      includeImages: false,
    };

    expect(buildCreateJobPayload(single)).toEqual({
      sourceUrl: "https://example.test/single-page/index.html",
      mode: "single",
      metadata: {
        title: "Single Book",
        author: "A. Reader",
        language: "en-US",
        description: "Single description",
      },
      options: {
        includeImages: false,
        outputTarget: "epub",
      },
    });

    const crawl = {
      ...single,
      mode: "crawl" as const,
      sourceUrl: "https://example.test/crawl-graph/index.html",
      crawlPrefixUrl: "https://example.test/crawl-graph/",
      maxDepth: "2",
      maxPages: "7",
    };

    expect(buildCreateJobPayload(crawl)).toMatchObject({
      sourceUrl: "https://example.test/crawl-graph/index.html",
      mode: "crawl",
      crawl: {
        prefixUrl: "https://example.test/crawl-graph/",
        maxDepth: 2,
        maxPages: 7,
        maxTotalBytes: 10485760,
        maxDurationMillis: 30000,
      },
    });
  });

  it("prevents duplicate submissions while a job is active", () => {
    expect(canSubmitInPhase("idle")).toBe(true);
    expect(canSubmitInPhase("completed")).toBe(true);
    expect(canSubmitInPhase("failed")).toBe(true);
    expect(canSubmitInPhase("submitting")).toBe(false);
    expect(canSubmitInPhase("polling")).toBe(false);
  });
});
