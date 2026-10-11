import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { mkdtempSync, mkdirSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { KbFsError, listKbFiles, readKbFile, resolveDevKbRoot } from "./kbFs";

let base: string;
let root: string;
let outside: string;

function put(rel: string, content = "") {
  const abs = path.join(root, ...rel.split("/"));
  mkdirSync(path.dirname(abs), { recursive: true });
  writeFileSync(abs, content);
}

beforeAll(() => {
  base = realpathSync(mkdtempSync(path.join(tmpdir(), "dise-kbfs-")));
  root = path.join(base, "kb");
  outside = path.join(base, "outside");
  mkdirSync(root);
  mkdirSync(outside);
  writeFileSync(path.join(outside, "secret.md"), "outside");
  mkdirSync(path.join(outside, "dir"));
  writeFileSync(path.join(outside, "dir", "x.md"), "outside dir");

  put("self/底色.md", "# 底色\n");
  put("wiki/概念/甲.md", "# 甲\n");
  put("index.md", "# 目录\n");
  put("inbox/.gitkeep");
  put("录音.mp3", "binary");
  put(".git/HEAD.md", "no");
  put(".obsidian/workspace.md", "no");
  put("node_modules/pkg/README.md", "no");
  put("podcast/.venv/lib/LICENSE.md", "no");
  put("notes.txt", "no");
  symlinkSync(path.join(outside, "secret.md"), path.join(root, "escape.md"));
  symlinkSync(path.join(outside, "dir"), path.join(root, "linked-dir"));
  symlinkSync(path.join(root, "index.md"), path.join(root, "alias.md"));
  symlinkSync(path.join(root, ".git", "HEAD.md"), path.join(root, "git-link.md"));
});

afterAll(() => {
  rmSync(base, { recursive: true, force: true });
});

describe("resolveDevKbRoot", () => {
  it("prefers DISE_KB_PATH and expands ~", () => {
    expect(resolveDevKbRoot({ DISE_KB_PATH: "/tmp/kb" }, "/Users/a")).toBe("/tmp/kb");
    expect(resolveDevKbRoot({ DISE_KB_PATH: "~/kb" }, "/Users/a")).toBe("/Users/a/kb");
  });

  it("falls back to the default folder in the home directory", () => {
    expect(resolveDevKbRoot({}, "/Users/a")).toBe("/Users/a/Desktop/GHJProject/阅读");
    expect(resolveDevKbRoot({ DISE_KB_PATH: "  " }, "/Users/a")).toBe(
      "/Users/a/Desktop/GHJProject/阅读",
    );
  });
});

describe("listKbFiles", () => {
  it("lists markdown files with POSIX paths, skipping hidden and vendored folders", async () => {
    const files = await listKbFiles(root);
    expect(files.map((f) => f.path)).toEqual([
      "alias.md",
      "index.md",
      "self/底色.md",
      "wiki/概念/甲.md",
    ]);
    for (const file of files) expect(typeof file.mtime).toBe("number");
  });
});

describe("readKbFile", () => {
  it("reads files inside the KB", async () => {
    expect(await readKbFile(root, "self/底色.md")).toBe("# 底色\n");
    expect(await readKbFile(root, "alias.md")).toBe("# 目录\n");
  });

  it.each([
    ["../outside/secret.md", 400],
    ["/etc/hosts.md", 400],
    [".git/HEAD.md", 400],
    [".obsidian/workspace.md", 400],
    ["notes.txt", 400],
    ["escape.md", 403],
    ["linked-dir/x.md", 403],
    ["git-link.md", 403],
    ["self/没有.md", 404],
  ])("rejects %s with %i", async (rel, status) => {
    const error = await readKbFile(root, rel).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(KbFsError);
    expect((error as KbFsError).status).toBe(status);
  });
});
