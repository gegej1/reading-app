import { assertCommitMessage, assertKbPath } from "./kbPath";
import { KbFileExistsError, type KbFileEntry, type KbStore } from "./types";

export interface MemoryCommit {
  path: string;
  message: string;
}

export interface MemoryStore extends KbStore {
  kind: "memory";
  /** Writes recorded in order, like a tiny git log. */
  readonly commits: readonly MemoryCommit[];
  /** Current files as a plain object. */
  snapshot(): Record<string, string>;
}

/** An in-memory KB for tests and the demo. Writes stay in memory. */
export function createMemoryStore(
  seed: Readonly<Record<string, string>>,
  options: { now?: () => number } = {},
): MemoryStore {
  const now = options.now ?? (() => Date.now());
  const start = now();
  const files = new Map<string, { content: string; mtime: number }>(
    Object.entries(seed).map(([path, content]) => [path, { content, mtime: start }]),
  );
  const commits: MemoryCommit[] = [];

  return {
    kind: "memory",
    commits,
    async list(): Promise<KbFileEntry[]> {
      return [...files.entries()]
        .map(([path, file]) => ({ path, mtime: file.mtime }))
        .sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
    },
    async read(path: string): Promise<string> {
      const file = files.get(path);
      if (!file) throw new Error(`找不到文件：${path}`);
      return file.content;
    },
    async write(path: string, content: string, message: string): Promise<void> {
      assertKbPath(path);
      assertCommitMessage(message);
      files.set(path, { content, mtime: now() });
      commits.push({ path, message });
    },
    async create(path: string, content: string, message: string): Promise<void> {
      assertKbPath(path);
      assertCommitMessage(message);
      if (files.has(path)) throw new KbFileExistsError(path);
      files.set(path, { content, mtime: now() });
      commits.push({ path, message });
    },
    snapshot() {
      return Object.fromEntries(
        [...files.entries()].map(([path, file]) => [path, file.content]),
      );
    },
  };
}
