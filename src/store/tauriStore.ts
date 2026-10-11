import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { KbFileExistsError, type KbFileEntry, type KbStore } from "./types";

/** Signature of `invoke` from @tauri-apps/api/core (injectable for tests). */
export type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

/** What `kb_root` returns (see src-tauri/src/commands.rs). */
export interface KbRootInfo {
  path: string;
  source: "env" | "config" | "default";
  exists: boolean;
}

/** How `kb_create` starts its message when the file exists (see src-tauri/src/kb/ops.rs). */
const EXISTS_PREFIX = "文件已存在：";

/** What `kb_write` and `kb_create` return. */
export interface KbWriteOutcome {
  committed: boolean;
  commit: string | null;
}

export interface TauriStore extends KbStore {
  kind: "tauri";
  /** Where the desktop app found the knowledge base. */
  root(): Promise<KbRootInfo>;
}

/** Tauri errors arrive as plain strings; turn them into Error objects. */
function asError(error: unknown): Error {
  if (error instanceof Error) return error;
  return new Error(typeof error === "string" ? error : JSON.stringify(error));
}

/** Store backed by the Rust commands kb_list / kb_read / kb_write / kb_create / kb_root. */
export function createTauriStore(invoke: Invoke = tauriInvoke): TauriStore {
  const call = async <T>(command: string, args?: Record<string, unknown>) => {
    try {
      return await invoke<T>(command, args);
    } catch (error) {
      throw asError(error);
    }
  };

  return {
    kind: "tauri",
    async list(): Promise<KbFileEntry[]> {
      const entries = await call<{ path: string; mtime: number | null }[]>("kb_list");
      return entries.map(({ path, mtime }) =>
        typeof mtime === "number" ? { path, mtime } : { path },
      );
    },
    read(path: string): Promise<string> {
      return call<string>("kb_read", { path });
    },
    async write(path: string, content: string, message: string): Promise<void> {
      await call<KbWriteOutcome>("kb_write", { path, content, message });
    },
    async create(path: string, content: string, message: string): Promise<void> {
      try {
        await call<KbWriteOutcome>("kb_create", { path, content, message });
      } catch (error) {
        if (error instanceof Error && error.message.startsWith(EXISTS_PREFIX)) throw new KbFileExistsError(path);
        throw error;
      }
    },
    root(): Promise<KbRootInfo> {
      return call<KbRootInfo>("kb_root");
    },
  };
}
