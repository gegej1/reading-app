// Keeps the iPhone export up to date (desktop app, main window only): after
// every load of the knowledge base, a moment later, the pages are built
// from the 底色 layer and handed to the backend, which writes them into the
// bridge folder's 阅读/ (see src-tauri/src/bridge/export.rs). Again shortly
// after midnight, so 今日 shows the new day's principle.

import type { KbModel } from "../domain";
import type { Desktop } from "./desktop";

/** Wait this long after a load: a burst of changes leads to one export. */
export const EXPORT_DELAY_MS = 1500;

export interface ExportResult {
  exported: boolean;
  written: number;
  unchanged: number;
  removed: number;
}

export interface PhoneExporter {
  /** The knowledge base was (re)loaded. */
  loaded(model: KbModel): void;
  stop(): void;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** Milliseconds until a minute past the next local midnight. */
export function untilNextDay(now: Date): number {
  const next = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1, 0, 1, 0, 0);
  return Math.max(60_000, next.getTime() - now.getTime());
}

export function createPhoneExporter(
  bridge: Desktop,
  options: { delayMs?: number; now?: () => Date } = {},
): PhoneExporter {
  const delay = options.delayMs ?? EXPORT_DELAY_MS;
  const now = options.now ?? (() => new Date());
  let latest: KbModel | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let midnight: ReturnType<typeof setTimeout> | undefined;
  let running = false;
  let again = false;
  let stopped = false;
  let unlisten: (() => void) | null = null;

  const run = async () => {
    if (!latest || stopped) return;
    if (running) {
      again = true;
      return;
    }
    running = true;
    const model = latest;
    try {
      // Loaded on demand: the page renderer is only needed for the export.
      const { buildPhoneExport } = await import("../export/phoneExport");
      const files = buildPhoneExport(
        model.docs.map((doc) => ({ path: doc.path, content: doc.content })),
        now(),
      );
      await bridge.call<ExportResult>("bridge_export", { files });
    } catch (error) {
      bridge.log("warn", `iphone export failed: ${errorText(error)}`);
    } finally {
      running = false;
      if (again && !stopped) {
        again = false;
        schedule();
      }
    }
  };

  const schedule = () => {
    clearTimeout(timer);
    timer = setTimeout(() => void run(), delay);
  };

  const scheduleMidnight = () => {
    clearTimeout(midnight);
    midnight = setTimeout(() => {
      schedule();
      scheduleMidnight();
    }, untilNextDay(now()));
  };

  return {
    loaded(model) {
      if (!bridge.active || stopped) return;
      latest = model;
      schedule();
      if (midnight === undefined) scheduleMidnight();
      // The bridge folder became ready (startup, or switched on in the
      // settings): export what is loaded right away.
      unlisten ??= bridge.listen("bridge-ready", () => schedule());
    },
    stop() {
      stopped = true;
      clearTimeout(timer);
      clearTimeout(midnight);
      unlisten?.();
    },
  };
}
