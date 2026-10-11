import { describe, expect, it } from "vitest";
import { createMemoryStore } from "./memoryStore";
import { KbFileExistsError } from "./types";

describe("createMemoryStore", () => {
  const files = {
    "self/底色.md": "# 底色\n",
    "wiki/概念/甲.md": "# 甲\n",
  };

  it("lists files sorted by path with an mtime", async () => {
    const store = createMemoryStore(files, { now: () => 1000 });
    expect(store.kind).toBe("memory");
    expect(await store.list()).toEqual([
      { path: "self/底色.md", mtime: 1000 },
      { path: "wiki/概念/甲.md", mtime: 1000 },
    ]);
  });

  it("reads files and rejects unknown ones", async () => {
    const store = createMemoryStore(files);
    expect(await store.read("self/底色.md")).toBe("# 底色\n");
    await expect(store.read("self/没有.md")).rejects.toThrow(/self\/没有\.md/);
  });

  it("writes content, bumps the mtime and records a commit", async () => {
    let clock = 1000;
    const store = createMemoryStore(files, { now: () => clock });
    clock = 2000;
    await store.write("inbox/新的.md", "新内容", "收件箱：新增");
    expect(await store.read("inbox/新的.md")).toBe("新内容");
    expect((await store.list()).find((f) => f.path === "inbox/新的.md")).toEqual({
      path: "inbox/新的.md",
      mtime: 2000,
    });
    expect(store.commits).toEqual([{ path: "inbox/新的.md", message: "收件箱：新增" }]);
  });

  it("does not mutate the seed object", async () => {
    const seed = { "a.md": "1" };
    const store = createMemoryStore(seed);
    await store.write("a.md", "2", "改");
    expect(seed["a.md"]).toBe("1");
    expect(store.snapshot()).toEqual({ "a.md": "2" });
  });

  it("creates new files but never overwrites", async () => {
    let clock = 1000;
    const store = createMemoryStore(files, { now: () => clock });
    clock = 3000;
    await store.create("inbox/2026-10-12T14-03-22-web.md", "一条\n", "收件箱：记一笔（web）");
    expect(await store.read("inbox/2026-10-12T14-03-22-web.md")).toBe("一条\n");
    expect(store.commits).toEqual([{ path: "inbox/2026-10-12T14-03-22-web.md", message: "收件箱：记一笔（web）" }]);

    const exists = store.create("self/底色.md", "覆盖？", "不行");
    await expect(exists).rejects.toBeInstanceOf(KbFileExistsError);
    await expect(store.create("self/底色.md", "覆盖？", "不行")).rejects.toMatchObject({ code: "exists" });
    expect(await store.read("self/底色.md")).toBe("# 底色\n");
    expect(store.commits).toHaveLength(1);
    await expect(store.create("../x.md", "x", "m")).rejects.toThrow(/路径不安全/);
    await expect(store.create("inbox/y.md", "x", " ")).rejects.toThrow(/提交说明/);
  });

  it("applies the same path rules as the real stores", async () => {
    const store = createMemoryStore(files);
    await expect(store.write("../逃逸.md", "x", "m")).rejects.toThrow();
    await expect(store.write(".git/config.md", "x", "m")).rejects.toThrow();
    await expect(store.write("a.txt", "x", "m")).rejects.toThrow();
    await expect(store.write("a.md", "x", "  ")).rejects.toThrow(/提交说明/);
  });
});
