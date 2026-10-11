// Node-side, read-only access to a local knowledge base, used by the
// `pnpm dev:kb` middleware and by the local real-KB compatibility test.
// It applies the same path rules as the Tauri commands (src-tauri/src/kb).
// Never imported by app code, so it never ends up in a build.

import { promises as fs, type Dirent } from "node:fs";
import os from "node:os";
import path from "node:path";
import { checkKbPath, isInsideRoot } from "../src/store/kbPath";

/** Default KB location relative to the home directory. */
export const DEFAULT_KB_RELATIVE = "Desktop/GHJProject/阅读";

/** Folders never listed (hidden folders are skipped as well). */
const SKIPPED_DIRS = new Set(["node_modules"]);

export class KbFsError extends Error {
  /** HTTP status the dev server answers with. */
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = "KbFsError";
    this.status = status;
  }
}

export interface KbFsEntry {
  path: string;
  mtime: number;
}

/** DISE_KB_PATH (with ~ expanded) or ~/Desktop/GHJProject/阅读. */
export function resolveDevKbRoot(
  env: Record<string, string | undefined> = process.env,
  home: string = os.homedir(),
): string {
  const fromEnv = env.DISE_KB_PATH?.trim();
  if (fromEnv) {
    if (fromEnv === "~") return home;
    if (fromEnv.startsWith("~/")) return path.join(home, fromEnv.slice(2));
    return fromEnv;
  }
  return path.join(home, DEFAULT_KB_RELATIVE);
}

async function realRoot(root: string): Promise<string> {
  try {
    const real = await fs.realpath(root);
    if (!(await fs.stat(real)).isDirectory()) throw new Error("not a directory");
    return real;
  } catch {
    throw new KbFsError(500, `找不到知识库文件夹：${root}（可以用环境变量 DISE_KB_PATH 指定）`);
  }
}

function toPosix(relative: string): string {
  return relative.split(path.sep).join("/");
}

/** Every `*.md` file under the root as a sorted repo-relative POSIX path. */
export async function listKbFiles(root: string): Promise<KbFsEntry[]> {
  const base = await realRoot(root);
  const found: KbFsEntry[] = [];

  const walk = async (dir: string): Promise<void> => {
    let entries: Dirent[];
    try {
      entries = await fs.readdir(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const name = entry.name;
      if (name.startsWith(".") || SKIPPED_DIRS.has(name)) continue;
      const abs = path.join(dir, name);
      if (entry.isDirectory()) {
        await walk(abs);
        continue;
      }
      if (!name.endsWith(".md")) continue;
      const rel = toPosix(path.relative(base, abs));
      if (!checkKbPath(rel).ok) continue;
      if (entry.isFile()) {
        found.push({ path: rel, mtime: Math.round((await fs.lstat(abs)).mtimeMs) });
      } else if (entry.isSymbolicLink()) {
        // Symlinked files count only when they point to a Markdown file inside
        // the KB; symlinked folders are never followed.
        const target = await safeTarget(base, abs);
        if (target) found.push({ path: rel, mtime: Math.round(target.mtimeMs) });
      }
    }
  };

  await walk(base);
  return found.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
}

async function safeTarget(base: string, abs: string) {
  try {
    const real = await fs.realpath(abs);
    if (!isInsideRoot(base, real)) return null;
    if (!checkKbPath(toPosix(path.relative(base, real))).ok) return null;
    const stat = await fs.stat(real);
    return stat.isFile() ? stat : null;
  } catch {
    return null;
  }
}

/** Reads one file after checking the path and where it really points. */
export async function readKbFile(root: string, rel: string): Promise<string> {
  const check = checkKbPath(rel);
  if (!check.ok) throw new KbFsError(400, `${check.message}：${rel}`);
  const base = await realRoot(root);

  let real: string;
  try {
    real = await fs.realpath(path.join(base, ...rel.split("/")));
  } catch {
    throw new KbFsError(404, `找不到文件：${rel}`);
  }
  if (!isInsideRoot(base, real) || !checkKbPath(toPosix(path.relative(base, real))).ok) {
    throw new KbFsError(403, `路径不安全：经符号链接指向了知识库外部或隐藏目录：${rel}`);
  }
  if (!(await fs.stat(real)).isFile()) throw new KbFsError(404, `不是文件：${rel}`);
  return fs.readFile(real, "utf8");
}
