import { describe, expect, it } from "vitest";
import { createTauriStore, type Invoke } from "./tauriStore";
import { KbFileExistsError } from "./types";

function fakeInvoke(handlers: Record<string, (args?: Record<string, unknown>) => unknown>) {
  const calls: [string, Record<string, unknown> | undefined][] = [];
  const invoke: Invoke = async <T>(command: string, args?: Record<string, unknown>) => {
    calls.push([command, args]);
    const handler = handlers[command];
    if (!handler) throw `unknown command ${command}`;
    return (await handler(args)) as T;
  };
  return { invoke, calls };
}

describe("createTauriStore", () => {
  it("maps list/read/write/root onto the Rust commands", async () => {
    const { invoke, calls } = fakeInvoke({
      kb_list: () => [
        { path: "self/底色.md", mtime: 5 },
        { path: "index.md", mtime: null },
      ],
      kb_read: (args) => `内容 ${String(args?.path)}`,
      kb_write: () => ({ committed: true, commit: "abc" }),
      kb_root: () => ({ path: "/kb", source: "env", exists: true }),
    });
    const store = createTauriStore(invoke);

    expect(store.kind).toBe("tauri");
    expect(await store.list()).toEqual([
      { path: "self/底色.md", mtime: 5 },
      { path: "index.md" },
    ]);
    expect(await store.read("self/底色.md")).toBe("内容 self/底色.md");
    await store.write("self/底色.md", "新", "底色：确认 D01");
    expect(await store.root()).toEqual({ path: "/kb", source: "env", exists: true });

    expect(calls).toEqual([
      ["kb_list", undefined],
      ["kb_read", { path: "self/底色.md" }],
      ["kb_write", { path: "self/底色.md", content: "新", message: "底色：确认 D01" }],
      ["kb_root", undefined],
    ]);
  });

  it("creates files through kb_create and recognises a taken name", async () => {
    const { invoke, calls } = fakeInvoke({
      kb_create: (args) => {
        if (args?.path === "inbox/taken.md") throw "文件已存在：inbox/taken.md";
        if (args?.path === "self/x.md") throw "这个窗口只能在 inbox 里新建文件：self/x.md";
        return { committed: true, commit: "abc" };
      },
    });
    const store = createTauriStore(invoke);
    await store.create("inbox/new.md", "新\n", "收件箱：记一笔（mac）");
    expect(calls).toEqual([["kb_create", { path: "inbox/new.md", content: "新\n", message: "收件箱：记一笔（mac）" }]]);

    const taken = store.create("inbox/taken.md", "x", "m");
    await expect(taken).rejects.toBeInstanceOf(KbFileExistsError);
    await expect(store.create("inbox/taken.md", "x", "m")).rejects.toMatchObject({ code: "exists", path: "inbox/taken.md" });
    // Other refusals stay ordinary errors with Rust's message.
    const refused = store.create("self/x.md", "x", "m");
    await expect(refused).rejects.toThrow("只能在 inbox 里新建文件");
    await expect(store.create("self/x.md", "x", "m")).rejects.not.toBeInstanceOf(KbFileExistsError);
  });

  it("turns string errors from Rust into Error objects", async () => {
    const { invoke } = fakeInvoke({
      kb_read: () => {
        throw "路径不安全：不能使用 .. 段";
      },
    });
    const store = createTauriStore(invoke);
    await expect(store.read("../x.md")).rejects.toThrow("路径不安全");
    await expect(store.read("../x.md")).rejects.toBeInstanceOf(Error);
  });
});
