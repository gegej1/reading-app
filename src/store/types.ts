/** Where the app reads and writes the knowledge base. */
export type KbStoreKind = "tauri" | "dev" | "memory" | "github";

export interface KbFileEntry {
  /** Repo-relative POSIX path, e.g. `self/底色.md`. */
  path: string;
  /** Last modification time in milliseconds since the epoch, when known. */
  mtime?: number;
}

/**
 * Access to the knowledge base (a Markdown git repo). Implementations:
 * - tauri: Rust commands on the local repo (writes are committed with git);
 * - dev: read-only Vite dev-server middleware (`pnpm dev:kb`);
 * - memory: in-memory files for tests and the demo;
 * - github: GitHub REST API (later milestone).
 */
export interface KbStore {
  kind: KbStoreKind;
  /** All Markdown files in the KB. */
  list(): Promise<KbFileEntry[]>;
  /** UTF-8 content of one file. */
  read(path: string): Promise<string>;
  /** Replaces one file and records the change with `message` (a git commit for real stores). */
  write(path: string, content: string, message: string): Promise<void>;
  /**
   * Creates a new file and records it with `message`. Never overwrites: when
   * the file exists it fails with {@link KbFileExistsError}.
   */
  create(path: string, content: string, message: string): Promise<void>;
}

/** `create` found a file with that name already; nothing was changed. */
export class KbFileExistsError extends Error {
  readonly code = "exists";
  readonly path: string;
  constructor(path: string) {
    super(`文件已存在：${path}`);
    this.name = "KbFileExistsError";
    this.path = path;
  }
}
