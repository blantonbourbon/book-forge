import http from "node:http";
import dns from "node:dns/promises";
import net from "node:net";
import { launch } from "cloakbrowser";

process.env.CLOAKBROWSER_AUTO_UPDATE ??= "false";

const PORT = parseInt(process.env.CLOAK_PORT || "3102", 10);
const MAX_BYTES = parseInt(process.env.CLOAK_MAX_BYTES || String(10 * 1024 * 1024), 10);
const TIMEOUT_MS = parseInt(process.env.CLOAK_TIMEOUT_MS || "30000", 10);
const MAX_INFLIGHT = parseInt(process.env.CLOAK_MAX_INFLIGHT || "2", 10);
const MAX_BODY = 8_192;

const UNSAFE_URL_MESSAGE =
  "Private, local, link-local, metadata, and multicast targets are not allowed.";

let browser = null;
let browserPromise = null;
let inflight = 0;

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

function isBlockedHostname(hostname) {
  const host = hostname.toLowerCase().replace(/\.$/, "");
  if (host === "localhost" || host === "localhost.localdomain") return true;
  if (host.endsWith(".localhost") || host.endsWith(".localhost.localdomain")) return true;
  if (host === "metadata" || host === "metadata.google.internal" || host === "169.254.169.254") {
    return true;
  }
  return false;
}

function ensurePublicIPv4(ip) {
  const parts = ip.split(".").map((p) => parseInt(p, 10));
  if (parts.length !== 4 || parts.some((n) => Number.isNaN(n) || n < 0 || n > 255)) {
    return false;
  }
  const [a, b, c] = parts;

  // loopback 127/8, unspecified 0/8, private 10/8, 172.16-31/12, 192.168/16
  if (a === 127 || a === 0 || a === 10) return false;
  if (a === 172 && b >= 16 && b <= 31) return false;
  if (a === 192 && b === 168) return false;
  // link-local 169.254/16
  if (a === 169 && b === 254) return false;
  // CGNAT 100.64-127/10
  if (a === 100 && b >= 64 && b <= 127) return false;
  // IETF protocol assignments 192.0.0/24
  if (a === 192 && b === 0 && c === 0) return false;
  // benchmarking 198.18-19/15
  if (a === 198 && (b === 18 || b === 19)) return false;
  // multicast 224-239, reserved/experimental >= 240
  if (a >= 224) return false;

  return true;
}

function ensurePublicIPv6(ip) {
  // Strip zone id (e.g. fe80::1%eth0)
  const bare = ip.split("%")[0].toLowerCase();

  // Mapped IPv4 ::ffff:x.x.x.x
  const v4Mapped = bare.match(/^::ffff:(\d{1,3}(?:\.\d{1,3}){3})$/i);
  if (v4Mapped) return ensurePublicIPv4(v4Mapped[1]);

  // Expand abbreviated IPv6 enough for prefix checks via net.isIPv6 + hex groups
  const expanded = expandIPv6(bare);
  if (!expanded) return false;

  const segments = expanded.split(":").map((s) => parseInt(s, 16));
  const first = segments[0];

  // :: (unspecified) and ::1 (loopback)
  if (segments.every((s) => s === 0)) return false;
  if (segments.slice(0, 7).every((s) => s === 0) && segments[7] === 1) return false;

  // fe80::/10 link-local
  if ((first & 0xffc0) === 0xfe80) return false;
  // fc00::/7 unique local
  if ((first & 0xfe00) === 0xfc00) return false;
  // ff00::/8 multicast
  if ((first & 0xff00) === 0xff00) return false;

  return true;
}

function expandIPv6(ip) {
  if (!net.isIPv6(ip)) return null;
  let str = ip.toLowerCase();
  if (str.includes(".")) {
    // embedded IPv4 already handled by caller for ::ffff:; reject others as unsafe
    return null;
  }
  const sides = str.split("::");
  let groups;
  if (sides.length === 2) {
    const left = sides[0] ? sides[0].split(":") : [];
    const right = sides[1] ? sides[1].split(":") : [];
    const fill = 8 - left.length - right.length;
    if (fill < 0) return null;
    groups = [...left, ...Array(fill).fill("0"), ...right];
  } else if (sides.length === 1) {
    groups = str.split(":");
  } else {
    return null;
  }
  if (groups.length !== 8) return null;
  return groups.map((g) => g.padStart(4, "0")).join(":");
}

function ensurePublicIP(address) {
  if (net.isIPv4(address)) return ensurePublicIPv4(address);
  if (net.isIPv6(address)) return ensurePublicIPv6(address);
  return false;
}

/**
 * Validate a URL string is safe to fetch (scheme, hostname, and if IP literal, public IP).
 * Optionally perform DNS lookup for hostnames.
 * @returns {{ ok: true, parsed: URL } | { ok: false, error: object }}
 */
async function validatePublicURL(rawUrl, { resolveDNS = true } = {}) {
  let parsed;
  try {
    parsed = new URL(rawUrl);
  } catch {
    return {
      ok: false,
      error: { code: "unsafe_url", message: "Only HTTP and HTTPS source URLs are supported." },
    };
  }

  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    return {
      ok: false,
      error: { code: "unsafe_url", message: "Only HTTP and HTTPS source URLs are supported." },
    };
  }

  const hostname = parsed.hostname;
  if (!hostname) {
    return {
      ok: false,
      error: { code: "unsafe_url", message: "Source URLs must include a host." },
    };
  }

  if (isBlockedHostname(hostname)) {
    return {
      ok: false,
      error: { code: "unsafe_url", message: "Localhost and metadata-service targets are not allowed." },
    };
  }

  if (net.isIP(hostname)) {
    if (!ensurePublicIP(hostname)) {
      return { ok: false, error: { code: "unsafe_url", message: UNSAFE_URL_MESSAGE } };
    }
    return { ok: true, parsed };
  }

  if (resolveDNS) {
    try {
      const { address } = await dns.lookup(hostname);
      if (!ensurePublicIP(address)) {
        return { ok: false, error: { code: "unsafe_url", message: UNSAFE_URL_MESSAGE } };
      }
    } catch {
      return {
        ok: false,
        error: {
          code: "unsafe_url",
          message: "DNS lookup did not complete safely for the requested host.",
        },
      };
    }
  }

  return { ok: true, parsed };
}

async function fetchPage(url) {
  const pre = await validatePublicURL(url, { resolveDNS: true });
  if (!pre.ok) return pre;

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
    // Re-validate final URL (redirect target): scheme/host/IP + DNS for non-IP hosts
    const post = await validatePublicURL(finalUrl, { resolveDNS: true });
    if (!post.ok) {
      // Do not return HTML for unsafe final destinations
      return post;
    }

    const html = await page.content();
    const bytes = Buffer.byteLength(html, "utf-8");

    if (bytes > MAX_BYTES) {
      return {
        ok: false,
        error: {
          code: "response_too_large",
          message: "Fetched content exceeded the configured byte limit.",
        },
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

function statusForResult(result) {
  if (result.ok) return 200;
  const code = result.error?.code;
  if (code === "response_too_large") return 413;
  if (code === "busy") return 503;
  return 400;
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
  let bodyTooLarge = false;

  req.on("data", (chunk) => {
    if (bodyTooLarge) return;
    body += chunk;
    if (body.length > MAX_BODY) {
      bodyTooLarge = true;
      body = "";
      if (!res.headersSent) {
        sendJSON(res, 413, {
          ok: false,
          error: { code: "request_too_large", message: "Request body exceeded the size limit." },
        });
      }
      req.destroy();
    }
  });

  req.on("end", async () => {
    if (bodyTooLarge) return;

    if (inflight >= MAX_INFLIGHT) {
      sendJSON(res, 503, {
        ok: false,
        error: { code: "busy", message: "Sidecar is busy; try again shortly." },
      });
      return;
    }

    inflight += 1;
    try {
      let parsedBody;
      try {
        parsedBody = JSON.parse(body);
      } catch {
        sendJSON(res, 400, {
          ok: false,
          error: { code: "invalid_request", message: "A URL is required." },
        });
        return;
      }

      const { url } = parsedBody;
      if (!url || typeof url !== "string") {
        sendJSON(res, 400, {
          ok: false,
          error: { code: "invalid_request", message: "A URL is required." },
        });
        return;
      }

      const result = await fetchPage(url);
      sendJSON(res, statusForResult(result), result);
    } catch (err) {
      console.error("Fetch error:", err.message);
      sendJSON(res, 500, {
        ok: false,
        error: { code: "fetch_failed", message: "Browser fetch failed." },
      });
    } finally {
      inflight -= 1;
    }
  });

  req.on("error", () => {
    // Connection aborted (e.g. after destroy on oversized body)
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
