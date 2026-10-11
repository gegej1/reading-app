// Desktop-only integrations: writing to the app log and opening links in the
// default browser. In the web build (and in tests) everything here is a no-op,
// so callers never have to check where they run.

import { invoke as tauriInvoke, isTauri } from "@tauri-apps/api/core";

export type LogLevel = "info" | "warn" | "error";

type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

/** tauri-plugin-log's numeric levels. */
const LEVELS: Record<LogLevel, number> = { info: 3, warn: 4, error: 5 };
/** Longest message sent to the log. */
const MAX_MESSAGE = 500;
const HTTP_URL = /^https?:\/\//i;

export interface Desktop {
  /** True inside the Tauri app. */
  readonly active: boolean;
  /** Writes one line to the app log. Never throws; does nothing on the web. */
  log(level: LogLevel, message: string): void;
  /** Opens an http(s) URL in the default browser (desktop only). */
  openUrl(url: string): Promise<void>;
}

/** Cuts long messages and keeps them on one line. */
export function logText(message: string): string {
  const line = message.replace(/\s*\n\s*/g, " ⏎ ").trim();
  return line.length > MAX_MESSAGE ? `${line.slice(0, MAX_MESSAGE)}…` : line;
}

export function createDesktop(options: { active: boolean; invoke?: Invoke }): Desktop {
  const call: Invoke = options.invoke ?? ((command, args) => tauriInvoke(command, args));
  const active = options.active;

  // The plugin commands are called directly (rather than through the plugins'
  // npm wrappers) so the log target stays short: `webview::app`.
  const log = (level: LogLevel, message: string) => {
    if (!active) return;
    // A failed log call is dropped: reporting it would only produce more failures.
    call("plugin:log|log", { level: LEVELS[level], message: logText(message), location: "app" }).catch(
      () => undefined,
    );
  };

  const openUrl = async (url: string) => {
    if (!active) throw new Error("只有桌面 App 能用默认浏览器打开链接");
    if (!HTTP_URL.test(url)) throw new Error(`只能打开 http(s) 链接：${url}`);
    try {
      await call("plugin:opener|open_url", { url });
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      log("warn", `open url failed: ${reason}`);
      throw error instanceof Error ? error : new Error(reason);
    }
  };

  return { active, log, openUrl };
}

/** The log line after a load, e.g. `kb loaded: files=52 brokenLinks=0 store=tauri`. */
export function kbLoadSummary(
  verb: "kb loaded" | "kb reloaded",
  model: { docs: readonly unknown[]; links: { broken: readonly unknown[] } },
  store: { kind: string },
): string {
  return `${verb}: files=${model.docs.length} brokenLinks=${model.links.broken.length} store=${store.kind}`;
}

/** The desktop bridge for this runtime. */
export const desktop: Desktop = createDesktop({ active: isTauri() });

/** One log line describing an uncaught error (`window.onerror`). */
export function describeError(event: Pick<ErrorEvent, "message" | "filename" | "lineno" | "colno" | "error">) {
  const error: unknown = event.error;
  const message = error instanceof Error ? `${error.name}: ${error.message}` : event.message || "unknown error";
  const where = event.filename ? ` at ${event.filename}:${event.lineno}:${event.colno}` : "";
  return `uncaught error: ${message}${where}`;
}

/** One log line describing an unhandled promise rejection. */
export function describeRejection(reason: unknown): string {
  if (reason instanceof Error) return `unhandled rejection: ${reason.name}: ${reason.message}`;
  if (typeof reason === "string") return `unhandled rejection: ${reason}`;
  try {
    return `unhandled rejection: ${JSON.stringify(reason)}`;
  } catch {
    return `unhandled rejection: ${String(reason)}`;
  }
}

/** One log line describing something the Content-Security-Policy blocked. */
export function describeCspViolation(
  event: Pick<SecurityPolicyViolationEvent, "effectiveDirective" | "blockedURI" | "sourceFile" | "lineNumber">,
): string {
  const where = event.sourceFile ? ` at ${event.sourceFile}:${event.lineNumber}` : "";
  return `CSP blocked ${event.effectiveDirective}: ${event.blockedURI || "inline"}${where}`;
}

type EventSource = Pick<Window, "addEventListener" | "removeEventListener">;

/**
 * Forwards uncaught errors, unhandled rejections and CSP violations to the app
 * log. Desktop only; returns a function that stops forwarding.
 */
export function forwardErrorsToLog(bridge: Desktop = desktop, target: EventSource = window): () => void {
  if (!bridge.active) return () => undefined;
  const onError = (event: Event) => bridge.log("error", describeError(event as ErrorEvent));
  const onRejection = (event: Event) =>
    bridge.log("error", describeRejection((event as PromiseRejectionEvent).reason));
  const onCsp = (event: Event) =>
    bridge.log("error", describeCspViolation(event as SecurityPolicyViolationEvent));
  target.addEventListener("error", onError);
  target.addEventListener("unhandledrejection", onRejection);
  target.addEventListener("securitypolicyviolation", onCsp);
  return () => {
    target.removeEventListener("error", onError);
    target.removeEventListener("unhandledrejection", onRejection);
    target.removeEventListener("securitypolicyviolation", onCsp);
  };
}

/**
 * Click handler of external links. In the desktop app the link opens in the
 * default browser; on the web the browser follows `target="_blank"` itself.
 */
export function openExternalLink(
  event: { preventDefault(): void },
  href: string,
  bridge: Desktop = desktop,
): void {
  if (!bridge.active) return;
  event.preventDefault();
  bridge.openUrl(href).catch(() => undefined);
}
