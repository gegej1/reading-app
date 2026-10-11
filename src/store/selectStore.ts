import { isTauri } from "@tauri-apps/api/core";
import type { KbStore } from "./types";

export type StoreChoice = "tauri" | "dev" | "memory";

export interface StoreEnvironment {
  /** Running inside the Tauri webview. */
  isTauri: boolean;
  /** Vite mode (`import.meta.env.MODE`). */
  mode: string;
}

/** Tauri runtime -> tauri; `vite --mode kb` -> dev; otherwise the demo memory store. */
export function chooseStoreKind(env: StoreEnvironment): StoreChoice {
  if (env.isTauri) return "tauri";
  if (env.mode === "kb") return "dev";
  return "memory";
}

export function detectEnvironment(): StoreEnvironment {
  return { isTauri: isTauri(), mode: import.meta.env.MODE };
}

/**
 * Creates the store for this runtime. Implementations are loaded lazily so a
 * build only pulls in what its runtime can use (the demo data, for example).
 */
export async function createStore(env: StoreEnvironment = detectEnvironment()): Promise<KbStore> {
  switch (chooseStoreKind(env)) {
    case "tauri": {
      const { createTauriStore } = await import("./tauriStore");
      return createTauriStore();
    }
    case "dev": {
      // `import.meta.env.DEV` is a build-time constant: production builds drop
      // this branch and never contain the dev store.
      return import.meta.env.DEV
        ? (await import("./devStore")).createDevStore()
        : Promise.reject(new Error("开发预览只在 pnpm dev:kb 下可用"));
    }
    case "memory": {
      const [{ createMemoryStore }, { demoKb }] = await Promise.all([
        import("./memoryStore"),
        import("../demo/demoKb"),
      ]);
      return createMemoryStore(demoKb);
    }
  }
}

/** The dev preview cannot write; every other store can. */
export function canWrite(store: Pick<KbStore, "kind">): boolean {
  return store.kind !== "dev";
}

/** Short Chinese label for where the data comes from. */
export function storeLabel(store: Pick<KbStore, "kind">): string {
  switch (store.kind) {
    case "tauri":
      return "本机知识库";
    case "dev":
      return "开发预览 · 只读";
    case "memory":
      return "演示数据";
    case "github":
      return "GitHub 知识库";
  }
}
