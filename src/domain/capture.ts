// Quick captures (记一笔): one new file in `inbox/` per note, in the format
// the KB's AGENTS.md describes (YAML with 来源 and 时间, then the text).
//
// The Rust side builds the same files for `--capture` on the command line
// (src-tauri/src/kb/capture.rs); both are tested against
// __fixtures__/capture-cases.json.

/** Where a capture was made. */
export type CaptureSource = "mac" | "iphone" | "web";

export const CAPTURE_SOURCES: readonly CaptureSource[] = ["mac", "iphone", "web"];

/** A wall-clock time with its offset from UTC. */
export interface LocalTime {
  year: number;
  /** 1–12. */
  month: number;
  day: number;
  hour: number;
  minute: number;
  second: number;
  /** Minutes east of UTC (480 for +08:00). */
  offsetMinutes: number;
}

/** A capture ready to be created in the knowledge base. */
export interface Capture {
  path: string;
  content: string;
  message: string;
}

/** How many names are tried when captures collide within one second. */
export const MAX_CAPTURE_ATTEMPTS = 20;

const pad = (value: number, width = 2) => String(value).padStart(width, "0");

/** The local time of `date` in this runtime's time zone. */
export function localTime(date: Date): LocalTime {
  return {
    year: date.getFullYear(),
    month: date.getMonth() + 1,
    day: date.getDate(),
    hour: date.getHours(),
    minute: date.getMinutes(),
    second: date.getSeconds(),
    offsetMinutes: -date.getTimezoneOffset(),
  };
}

/** `2026-10-12T14:03:22+08:00`. */
function isoLocal(at: LocalTime): string {
  const sign = at.offsetMinutes < 0 ? "-" : "+";
  const offset = Math.abs(at.offsetMinutes);
  return `${pad(at.year, 4)}-${pad(at.month)}-${pad(at.day)}T${pad(at.hour)}:${pad(at.minute)}:${pad(at.second)}${sign}${pad(Math.floor(offset / 60))}:${pad(offset % 60)}`;
}

/** True when there is nothing to save (empty, or only whitespace). */
export function isBlankCapture(text: string): boolean {
  return text.trim() === "";
}

/**
 * Builds the file for one capture made at `at`. The text is kept verbatim:
 * CRLF becomes LF and the file ends with exactly one newline; nothing inside
 * is trimmed or reflowed. Throws for empty text and unknown sources.
 */
export function buildCaptureAt({ text, source, at }: { text: string; source: CaptureSource; at: LocalTime }): Capture {
  if (!CAPTURE_SOURCES.includes(source)) throw new Error(`不认识的来源：${String(source)}`);
  if (isBlankCapture(text)) throw new Error("内容是空的，没有可以记下的东西");
  const body = text.replace(/\r\n/g, "\n").replace(/\n+$/, "");
  const stamp = `${pad(at.year, 4)}-${pad(at.month)}-${pad(at.day)}T${pad(at.hour)}-${pad(at.minute)}-${pad(at.second)}`;
  return {
    path: `inbox/${stamp}-${source}.md`,
    content: `---\n来源: ${source}\n时间: ${isoLocal(at)}\n---\n\n${body}\n`,
    message: `收件箱：记一笔（${source}）`,
  };
}

/** Builds the file for one capture made now (local time of `now`). */
export function buildCapture({ text, source, now }: { text: string; source: CaptureSource; now: Date }): Capture {
  return buildCaptureAt({ text, source, at: localTime(now) });
}

/** The name tried on the `attempt`-th try: `path` itself, then `-2`, `-3`, … before `.md`. */
export function captureAttemptPath(path: string, attempt: number): string {
  if (attempt <= 1) return path;
  return path.endsWith(".md") ? `${path.slice(0, -".md".length)}-${attempt}.md` : `${path}-${attempt}`;
}

/**
 * Where captures are created: any KbStore. `create` must fail when the file
 * exists, with an error whose `code` is `"exists"`.
 */
export interface CaptureTarget {
  create(path: string, content: string, message: string): Promise<unknown>;
}

/** True for the "file already exists" failure of `create`. */
export function isExistsError(error: unknown): boolean {
  return typeof error === "object" && error !== null && (error as { code?: unknown }).code === "exists";
}

export interface SavedCapture {
  path: string;
  /** 1 when the first name was free. */
  attempts: number;
}

/**
 * Creates the capture. When its name is taken (two captures within one
 * second), the next free name is used; nothing is ever overwritten. Other
 * failures are passed on as they are.
 */
export async function saveCapture(
  target: CaptureTarget,
  capture: Capture,
  options: { maxAttempts?: number } = {},
): Promise<SavedCapture> {
  const maxAttempts = options.maxAttempts ?? MAX_CAPTURE_ATTEMPTS;
  for (let attempt = 1; attempt <= maxAttempts; attempt += 1) {
    const path = captureAttemptPath(capture.path, attempt);
    try {
      await target.create(path, capture.content, capture.message);
      return { path, attempts: attempt };
    } catch (error) {
      if (!isExistsError(error)) throw error;
    }
  }
  throw new Error(`同一秒里已经有 ${maxAttempts} 条随手记了，这一条没有记下：${capture.path}`);
}
