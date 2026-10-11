// Vite dev-server plugin for `pnpm dev:kb`: serves the local knowledge base
// read-only at /__kb so the web UI can be developed against real files.
//
//   GET /__kb/list            -> [{ path, mtime }]
//   GET /__kb/read?path=...   -> file content (text/markdown)
//
// Dev only: `apply: "serve"` keeps it out of `vite build`, vite.config.ts
// registers it only in mode "kb", and app code never imports this file.

import type { Plugin } from "vite";
import { KbFsError, listKbFiles, readKbFile, resolveDevKbRoot } from "./kbFs";

export interface KbRequest {
  method?: string;
  /** URL relative to the /__kb mount point, e.g. `/read?path=...`. */
  url?: string;
  remoteAddress?: string;
  /** Host header. */
  host?: string;
}

export interface KbResponse {
  status: number;
  headers: Record<string, string>;
  body: string;
}

const LOOPBACK = new Set(["127.0.0.1", "::1", "::ffff:127.0.0.1"]);
const LOCAL_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]"]);

export function isLoopback(address: string | undefined): boolean {
  return address !== undefined && LOOPBACK.has(address);
}

/** Host header check against DNS rebinding: only local names are served. */
export function isLocalHostHeader(host: string | undefined): boolean {
  if (!host) return false;
  const name = host.startsWith("[") ? host.slice(0, host.indexOf("]") + 1) : host.split(":")[0];
  return LOCAL_HOSTS.has((name ?? "").toLowerCase());
}

function respond(status: number, body: string, contentType: string): KbResponse {
  return {
    status,
    headers: {
      "Content-Type": contentType,
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
    },
    body,
  };
}

const text = (status: number, message: string) =>
  respond(status, message, "text/plain; charset=utf-8");

/** Handles one request below /__kb. Never writes anything. */
export async function handleKbRequest(root: string, req: KbRequest): Promise<KbResponse> {
  if (!isLoopback(req.remoteAddress) || !isLocalHostHeader(req.host)) {
    return text(403, "只接受本机请求");
  }
  const method = (req.method ?? "GET").toUpperCase();
  if (method !== "GET" && method !== "HEAD") {
    return text(405, "只读：只支持 GET");
  }

  const url = new URL(req.url ?? "/", "http://localhost");
  try {
    if (url.pathname === "/list") {
      const files = await listKbFiles(root);
      return respond(200, JSON.stringify(files), "application/json; charset=utf-8");
    }
    if (url.pathname === "/read") {
      const rel = url.searchParams.get("path");
      if (!rel) return text(400, "缺少 path 参数");
      return respond(200, await readKbFile(root, rel), "text/markdown; charset=utf-8");
    }
    return text(404, "未知接口");
  } catch (error) {
    if (error instanceof KbFsError) return text(error.status, error.message);
    return text(500, error instanceof Error ? error.message : String(error));
  }
}

/** The plugin. `root` defaults to DISE_KB_PATH or ~/Desktop/GHJProject/阅读. */
export function kbDevServer(options: { root?: string } = {}): Plugin {
  return {
    name: "dise-kb-dev-server",
    apply: "serve",
    configureServer(server) {
      const root = options.root ?? resolveDevKbRoot();
      server.config.logger.info(`  [dev:kb] 只读知识库：${root}`);
      server.middlewares.use("/__kb", (req, res, next) => {
        handleKbRequest(root, {
          method: req.method,
          url: req.url,
          remoteAddress: req.socket.remoteAddress,
          host: req.headers.host,
        })
          .then((response) => {
            res.statusCode = response.status;
            for (const [name, value] of Object.entries(response.headers)) {
              res.setHeader(name, value);
            }
            res.end(req.method === "HEAD" ? undefined : response.body);
          })
          .catch(next);
      });
    },
  };
}
