import type { KbStore } from "../store/types";

export interface EditResult {
  next: string;
  message: string;
}

/**
 * Read-modify-write of one KB file. It re-reads the file right before editing,
 * so changes made outside the app since the last load are never overwritten.
 */
export async function editKbFile(
  store: Pick<KbStore, "read" | "write">,
  path: string,
  edit: (current: string) => string,
  message: (next: string) => string,
): Promise<EditResult> {
  const current = await store.read(path);
  const next = edit(current);
  const text = message(next);
  await store.write(path, next, text);
  return { next, message: text };
}
