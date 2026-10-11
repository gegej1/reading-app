// Path rules shared by every store and by the dev-server middleware. They mirror
// the Rust checks in src-tauri/src/kb/paths.rs: a KB path is a repo-relative POSIX
// path to a Markdown file outside `.git/` and outside hidden folders.

export type KbPathProblem =
  | "empty"
  | "absolute"
  | "traversal"
  | "invalid"
  | "git"
  | "hidden"
  | "not-markdown";

export type KbPathCheck =
  | { ok: true; path: string }
  | { ok: false; reason: KbPathProblem; message: string };

const MESSAGES: Record<KbPathProblem, string> = {
  empty: "路径为空",
  absolute: "不能使用绝对路径",
  traversal: "路径里不能有 . 或 .. 段",
  invalid: "路径格式不对",
  git: "不能访问 .git 目录",
  hidden: "不能访问隐藏文件或隐藏目录",
  "not-markdown": "只能访问 .md 文件",
};

function fail(reason: KbPathProblem): KbPathCheck {
  return { ok: false, reason, message: `路径不安全：${MESSAGES[reason]}` };
}

/** Validates a repo-relative KB path without touching the file system. */
export function checkKbPath(path: string): KbPathCheck {
  if (typeof path !== "string" || path === "") return fail("empty");
  if (path.startsWith("/") || path.startsWith("~") || /^[A-Za-z]:/.test(path)) {
    return fail("absolute");
  }
  if (path.includes("\\") || path.includes("\0")) return fail("invalid");

  const segments = path.split("/");
  for (const segment of segments) {
    if (segment === "." || segment === "..") return fail("traversal");
  }
  if (segments.some((segment) => segment === "")) return fail("invalid");
  if (segments.some((segment) => segment === ".git")) return fail("git");
  if (segments.some((segment) => segment.startsWith("."))) return fail("hidden");

  const file = segments[segments.length - 1] ?? "";
  if (!file.endsWith(".md") || file === ".md") return fail("not-markdown");
  return { ok: true, path };
}

/** Throws a readable error when the path is not a safe KB path. */
export function assertKbPath(path: string): string {
  const result = checkKbPath(path);
  if (!result.ok) throw new Error(`${result.message}（${String(path)}）`);
  return result.path;
}

/**
 * True when `resolved` lies strictly inside `root`. Both must already be
 * absolute, normalised POSIX paths (e.g. results of realpath).
 */
export function isInsideRoot(root: string, resolved: string): boolean {
  const base = root.endsWith("/") ? root : `${root}/`;
  return resolved.startsWith(base) && resolved.length > base.length;
}

/** Commit messages must say something. */
export function assertCommitMessage(message: string): string {
  if (typeof message !== "string" || message.trim() === "") {
    throw new Error("提交说明不能为空");
  }
  return message;
}
