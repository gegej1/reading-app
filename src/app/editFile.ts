import type { KbStore } from "../store/types";

export interface EditResult {
  /** False when the file already had the requested content: nothing was written or committed. */
  changed: boolean;
  /** The file content after the edit (the unchanged content when `changed` is false). */
  next: string;
  /** The commit message, or null when nothing was written. */
  message: string | null;
}

export type EditFn = (current: string) => string;
export type MessageFn = (next: string) => string;

/**
 * Read-modify-write of one KB file. It re-reads the file right before editing,
 * so changes made outside the app since the last load are never overwritten.
 * When the edit returns the content unchanged, nothing is written (so nothing
 * is committed either).
 *
 * Not safe for overlapping calls on its own: use {@link createKbEditor}.
 */
export async function editKbFile(
  store: Pick<KbStore, "read" | "write">,
  path: string,
  edit: EditFn,
  message: MessageFn,
): Promise<EditResult> {
  const current = await store.read(path);
  const next = edit(current);
  if (next === current) return { changed: false, next, message: null };
  const text = message(next);
  await store.write(path, next, text);
  return { changed: true, next, message: text };
}

/**
 * Runs async tasks strictly one after another, in call order. A task starts
 * only once the previous one has settled, whether it succeeded or failed.
 */
export function createSerialQueue() {
  let tail: Promise<unknown> = Promise.resolve();
  return function enqueue<T>(task: () => Promise<T>): Promise<T> {
    const run = tail.then(() => task());
    tail = run.then(
      () => undefined,
      () => undefined,
    );
    return run;
  };
}

export interface KbEditorOptions {
  /**
   * Runs after every edit attempt, success or failure, before the next queued
   * edit starts: the provider reloads here so the screen shows what is on disk.
   * Its errors are swallowed; it should report them itself.
   */
  afterEdit?: () => Promise<void>;
  /** Called whenever the number of queued or running edits changes. */
  onPendingChange?: (pending: number) => void;
}

export type KbEditor = (path: string, edit: EditFn, message: MessageFn) => Promise<EditResult>;

/**
 * The one way the app edits KB files: every edit goes through a single queue,
 * so two read-modify-write cycles never interleave (two quick clicks used to
 * let the second write restore what the first one replaced).
 */
export function createKbEditor(
  store: Pick<KbStore, "read" | "write">,
  options: KbEditorOptions = {},
): KbEditor {
  const enqueue = createSerialQueue();
  let pending = 0;
  const setPending = (count: number) => {
    pending = count;
    options.onPendingChange?.(count);
  };

  return async (path, edit, message) => {
    setPending(pending + 1);
    try {
      return await enqueue(async () => {
        try {
          return await editKbFile(store, path, edit, message);
        } finally {
          try {
            await options.afterEdit?.();
          } catch {
            // The edit's own outcome matters more than a failed reload.
          }
        }
      });
    } finally {
      setPending(pending - 1);
    }
  };
}
