import { describe, expect, it } from "vitest";
import { checkKbPath, isInsideRoot } from "./kbPath";

describe("checkKbPath", () => {
  it("accepts repo-relative markdown paths, including Chinese names", () => {
    for (const path of [
      "self/底色.md",
      "self/待裁决/Q-001-示例问题.md",
      "inbox/2026-10-12-想法.md",
      "某本书·读书笔记(整理稿).md",
      "a b/c d.md",
    ]) {
      expect(checkKbPath(path)).toEqual({ ok: true, path });
    }
  });

  it.each([
    ["", "empty"],
    ["/etc/passwd.md", "absolute"],
    ["~/notes.md", "absolute"],
    ["C:/notes.md", "absolute"],
    ["../outside.md", "traversal"],
    ["self/../../outside.md", "traversal"],
    ["./self/底色.md", "traversal"],
    ["self//底色.md", "invalid"],
    ["self/", "invalid"],
    ["self\\底色.md", "invalid"],
    ["self/底\0色.md", "invalid"],
    [".git/config.md", "git"],
    ["wiki/.git/x.md", "git"],
    [".obsidian/workspace.md", "hidden"],
    ["inbox/.draft.md", "hidden"],
    ["self/底色.txt", "not-markdown"],
    ["self/底色", "not-markdown"],
    ["self/底色.MD", "not-markdown"],
  ])("rejects %j (%s)", (path, reason) => {
    const result = checkKbPath(path);
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.reason).toBe(reason);
      expect(result.message.length).toBeGreaterThan(0);
    }
  });

  it("rejects non-string input", () => {
    expect(checkKbPath(undefined as unknown as string).ok).toBe(false);
  });
});

describe("isInsideRoot", () => {
  it("checks the resolved path is strictly inside the root", () => {
    expect(isInsideRoot("/kb", "/kb/self/底色.md")).toBe(true);
    expect(isInsideRoot("/kb/", "/kb/self/底色.md")).toBe(true);
    expect(isInsideRoot("/kb", "/kb")).toBe(false);
    expect(isInsideRoot("/kb", "/kb2/x.md")).toBe(false);
    expect(isInsideRoot("/kb", "/other/x.md")).toBe(false);
  });
});
