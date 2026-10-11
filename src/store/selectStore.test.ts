import { describe, expect, it } from "vitest";
import { canWrite, chooseStoreKind, createStore, storeLabel } from "./selectStore";

describe("chooseStoreKind", () => {
  it("prefers Tauri, then the kb dev mode, then the demo", () => {
    expect(chooseStoreKind({ isTauri: true, mode: "kb" })).toBe("tauri");
    expect(chooseStoreKind({ isTauri: true, mode: "production" })).toBe("tauri");
    expect(chooseStoreKind({ isTauri: false, mode: "kb" })).toBe("dev");
    expect(chooseStoreKind({ isTauri: false, mode: "development" })).toBe("memory");
    expect(chooseStoreKind({ isTauri: false, mode: "web" })).toBe("memory");
  });
});

describe("createStore", () => {
  it("builds the demo store with synthetic data outside Tauri and dev:kb", async () => {
    const store = await createStore({ isTauri: false, mode: "web" });
    expect(store.kind).toBe("memory");
    const paths = (await store.list()).map((f) => f.path);
    expect(paths).toContain("self/底色.md");
  });

  it("builds the read-only dev store in kb mode", async () => {
    const store = await createStore({ isTauri: false, mode: "kb" });
    expect(store.kind).toBe("dev");
    expect(canWrite(store)).toBe(false);
  });

  it("builds the Tauri store inside the desktop app", async () => {
    const store = await createStore({ isTauri: true, mode: "production" });
    expect(store.kind).toBe("tauri");
    expect(canWrite(store)).toBe(true);
  });
});

describe("storeLabel", () => {
  it("names every store kind", () => {
    expect(storeLabel({ kind: "memory" })).toBe("演示数据");
    expect(storeLabel({ kind: "dev" })).toMatch(/只读/);
    expect(storeLabel({ kind: "tauri" })).toBe("本机知识库");
    expect(storeLabel({ kind: "github" })).toMatch(/GitHub/);
  });
});
