import { describe, expect, it } from "vitest";

import {
  DEFAULT_API_ORIGIN,
  DEV_SERVER_PORT,
  FRONTEND_BOUNDARY,
} from "./config";

describe("frontend scaffold config", () => {
  it("names the Astro frontend boundary", () => {
    expect(FRONTEND_BOUNDARY).toBe("book-forge-frontend");
  });

  it("keeps local development on approved mission ports", () => {
    expect(DEFAULT_API_ORIGIN).toBe("http://127.0.0.1:3100");
    expect(DEV_SERVER_PORT).toBe(3101);
  });
});
