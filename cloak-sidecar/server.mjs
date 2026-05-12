import http from "node:http";
import { launch } from "cloakbrowser";

process.env.CLOAKBROWSER_AUTO_UPDATE ??= "false";

const PORT = parseInt(process.env.CLOAK_PORT || "3102", 10);
const MAX_BYTES = parseInt(process.env.CLOAK_MAX_BYTES || String(10 * 1024 * 1024), 10);
const TIMEOUT_MS = parseInt(process.env.CLOAK_TIMEOUT_MS || "30000", 10);

let browser = null;
let browserPromise = null;

async function getBrowser() {
  if (!browser) {
    if (!browserPromise) {
      browserPromise = launch({
        headless: true,
        args: [
          "--no-sandbox",
          "--disable-setuid-sandbox",
          "--disable-dev-shm-usage",
          "--disable-gpu",
        ],
      })
        .then((instance) => {
          browser = instance;
          console.log("CloakBrowser launched");
          return instance;
        })
        .catch((err) => {
          browserPromise = null;
          throw err;
        });
    }
    await browserPromise;
  }
  return browser;
}

async function fetchPage(url) {
  const instance = await getBrowser();
  const context = await instance.newContext({
    userAgent:
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    viewport: { width: 1366, height: 768 },
  });
  const page = await context.newPage();

  try {
    await page.goto(url, {
      waitUntil: "domcontentloaded",
      timeout: TIMEOUT_MS,
    });

    const finalUrl = page.url();
    const html = await page.content();
    const bytes = Buffer.byteLength(html, "utf-8");

    if (bytes > MAX_BYTES) {
      return {
        ok: false,
        error: { code: "response_too_large", message: "Fetched content exceeded the configured byte limit." },
      };
    }

    return {
      ok: true,
      html,
      finalUrl,
      mediaType: "text/html; charset=utf-8",
      bytes: bytes,
    };
  } finally {
    await context.close();
  }
}

function sendJSON(res, status, data) {
  const body = JSON.stringify(data);
  res.writeHead(status, {
    "Content-Type": "application/json",
    "Content-Length": Buffer.byteLength(body),
  });
  res.end(body);
}

const server = http.createServer(async (req, res) => {
  if (req.method === "GET" && req.url === "/health") {
    sendJSON(res, 200, {
      status: "healthy",
      browser: browser ? "ready" : browserPromise ? "starting" : "idle",
    });
    return;
  }

  if (req.method !== "POST" || req.url !== "/fetch") {
    sendJSON(res, 404, { error: { code: "not_found", message: "Not found" } });
    return;
  }

  let body = "";
  req.on("data", (chunk) => (body += chunk));
  req.on("end", async () => {
    try {
      const { url } = JSON.parse(body);
      if (!url || typeof url !== "string") {
        sendJSON(res, 400, { ok: false, error: { code: "invalid_request", message: "A URL is required." } });
        return;
      }

      const result = await fetchPage(url);
      const status = result.ok ? 200 : 413;
      sendJSON(res, status, result);
    } catch (err) {
      console.error("Fetch error:", err.message);
      sendJSON(res, 500, {
        ok: false,
        error: { code: "fetch_failed", message: err.message || "Browser fetch failed." },
      });
    }
  });
});

server.listen(PORT, "127.0.0.1", () => {
  console.log(`CloakBrowser sidecar listening on 127.0.0.1:${PORT}`);
  getBrowser().catch((err) => {
    console.error("CloakBrowser warmup failed:", err.message);
  });
});

process.on("SIGTERM", async () => {
  console.log("Shutting down CloakBrowser...");
  if (browser) await browser.close();
  process.exit(0);
});
