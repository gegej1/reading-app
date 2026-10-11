import { fencedCodeMask, splitLines } from "./markdown";

/** One `[[...]]` occurrence in a document. */
export interface WikiLink {
  /** Exact source text, e.g. `[[示例概念|密度]]`. */
  raw: string;
  /** Page name (basename or path without `.md`). Empty for same-page links like `[[#小节]]`. */
  target: string;
  heading: string | null;
  alias: string | null;
  /** `![[...]]` embed syntax. */
  embed: boolean;
  /** 0-based line index within the document. */
  line: number;
}

export interface WikiLinkParts {
  target: string;
  heading: string | null;
  alias: string | null;
}

/** Parses the text between `[[` and `]]`: `name`, `name|alias`, `name#heading|alias`. */
export function parseWikilinkInner(inner: string): WikiLinkParts {
  // `\|` is how a pipe is escaped inside Markdown table cells.
  const text = inner.replace(/\\\|/g, "|");
  const pipe = text.indexOf("|");
  const destination = pipe === -1 ? text : text.slice(0, pipe);
  const aliasText = pipe === -1 ? "" : text.slice(pipe + 1).trim();
  const hash = destination.indexOf("#");
  const target = (hash === -1 ? destination : destination.slice(0, hash))
    .trim()
    .replace(/\.md$/i, "");
  const heading = hash === -1 ? "" : destination.slice(hash + 1).trim();
  return {
    target,
    heading: heading === "" ? null : heading,
    alias: aliasText === "" ? null : aliasText,
  };
}

/**
 * Replaces inline code spans (backtick delimiters included) with spaces of the
 * same length, so offsets stay valid. Follows CommonMark: a run of N backticks is
 * closed by the next run of exactly N backticks; unmatched runs stay literal.
 */
export function maskInlineCode(text: string): string {
  const chars = text.split("");
  let i = 0;
  while (i < text.length) {
    if (text[i] !== "`") {
      i += 1;
      continue;
    }
    let runEnd = i;
    while (text[runEnd] === "`") runEnd += 1;
    const runLength = runEnd - i;
    if (isEscaped(text, i)) {
      i += 1;
      continue;
    }
    const close = findClosingRun(text, runEnd, runLength);
    if (close === -1) {
      i = runEnd;
      continue;
    }
    for (let j = i; j < close + runLength; j += 1) {
      if (chars[j] !== "\n") chars[j] = " ";
    }
    i = close + runLength;
  }
  return chars.join("");
}

function isEscaped(text: string, index: number): boolean {
  let backslashes = 0;
  for (let j = index - 1; j >= 0 && text[j] === "\\"; j -= 1) backslashes += 1;
  return backslashes % 2 === 1;
}

function findClosingRun(text: string, from: number, length: number): number {
  let i = from;
  while (i < text.length) {
    if (text[i] !== "`") {
      i += 1;
      continue;
    }
    let runEnd = i;
    while (text[runEnd] === "`") runEnd += 1;
    if (runEnd - i === length) return i;
    i = runEnd;
  }
  return -1;
}

const WIKILINK = /(!?)\[\[([^[\]\n]+?)\]\]/g;

/** Finds all wikilinks, skipping fenced code blocks and inline code spans. */
export function extractWikilinks(markdown: string): WikiLink[] {
  const lines = splitLines(markdown);
  const mask = fencedCodeMask(lines);
  const links: WikiLink[] = [];

  // Inline code is matched per paragraph (runs of non-blank, non-fenced lines),
  // because a code span cannot cross a blank line.
  let block: { start: number; lines: string[] } | null = null;
  const flush = () => {
    if (block) {
      collectLinks(block.lines, block.start, links);
      block = null;
    }
  };

  lines.forEach((line, index) => {
    if (mask[index] || line.trim() === "") {
      flush();
      return;
    }
    if (!block) block = { start: index, lines: [] };
    block.lines.push(line);
  });
  flush();
  return links;
}

function collectLinks(blockLines: string[], startLine: number, out: WikiLink[]) {
  const text = blockLines.join("\n");
  const masked = maskInlineCode(text);
  for (const match of masked.matchAll(WIKILINK)) {
    const index = match.index ?? 0;
    const raw = text.slice(index, index + match[0].length);
    const inner = raw.slice(raw.startsWith("!") ? 3 : 2, -2);
    const parts = parseWikilinkInner(inner);
    if (parts.target === "" && parts.heading === null) continue;
    out.push({
      raw,
      ...parts,
      embed: match[1] === "!",
      line: startLine + countNewlines(text, index),
    });
  }
}

function countNewlines(text: string, end: number): number {
  let count = 0;
  for (let i = 0; i < end; i += 1) if (text[i] === "\n") count += 1;
  return count;
}

/** `wiki/概念/示例概念.md` -> `示例概念`. */
export function basenameOf(path: string): string {
  const file = path.slice(path.lastIndexOf("/") + 1);
  return file.replace(/\.md$/i, "");
}

export interface LinkIndex {
  /** Basename -> paths sharing it, sorted. */
  byName: ReadonlyMap<string, readonly string[]>;
  /** Basenames used by more than one file (the KB requires unique names). */
  duplicates: { name: string; paths: string[] }[];
  /** Resolves a link target to a file path, or null when it is broken. */
  resolve(target: string): string | null;
}

const byCodeUnit = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

/** Builds the basename index used to resolve `[[name]]` links. */
export function buildLinkIndex(paths: readonly string[]): LinkIndex {
  const sorted = [...paths].sort(byCodeUnit);
  const byName = new Map<string, string[]>();
  const byLowerName = new Map<string, string>();
  for (const path of sorted) {
    const name = basenameOf(path);
    const list = byName.get(name);
    if (list) list.push(path);
    else byName.set(name, [path]);
    if (!byLowerName.has(name.toLowerCase())) byLowerName.set(name.toLowerCase(), path);
  }

  const duplicates = [...byName.entries()]
    .filter(([, list]) => list.length > 1)
    .map(([name, list]) => ({ name, paths: [...list] }));

  const resolve = (rawTarget: string): string | null => {
    const target = rawTarget.trim().replace(/\.md$/i, "");
    if (target === "") return null;
    if (target.includes("/")) {
      const exact = sorted.find(
        (path) => path === `${target}.md` || path.endsWith(`/${target}.md`),
      );
      if (exact) return exact;
    }
    const name = basenameOf(target);
    return byName.get(name)?.[0] ?? byLowerName.get(name.toLowerCase()) ?? null;
  };

  return { byName, duplicates, resolve };
}

export interface LinkRef {
  /** Path of the document that contains the link. */
  from: string;
  link: WikiLink;
}

export interface ResolvedLink {
  path: string;
  link: WikiLink;
}

export interface LinkAnalysis {
  /** Target path -> one entry per linking document, sorted by source path. */
  backlinks: Map<string, LinkRef[]>;
  /** Source path -> resolved targets in order of first appearance (deduplicated). */
  outgoing: Map<string, ResolvedLink[]>;
  /** Links whose target does not exist. */
  broken: LinkRef[];
}

/** Resolves every link in every document once. */
export function analyzeLinks(
  docs: readonly { path: string; content: string }[],
  index: LinkIndex,
): LinkAnalysis {
  const backlinks = new Map<string, LinkRef[]>();
  const outgoing = new Map<string, ResolvedLink[]>();
  const broken: LinkRef[] = [];
  const sortedDocs = [...docs].sort((a, b) => byCodeUnit(a.path, b.path));

  for (const doc of sortedDocs) {
    const resolvedHere: ResolvedLink[] = [];
    const seen = new Set<string>();
    for (const link of extractWikilinks(doc.content)) {
      if (link.target === "") continue; // same-page heading link
      const path = index.resolve(link.target);
      if (path === null) {
        broken.push({ from: doc.path, link });
        continue;
      }
      if (path === doc.path || seen.has(path)) continue;
      seen.add(path);
      resolvedHere.push({ path, link });
      const list = backlinks.get(path);
      const ref = { from: doc.path, link };
      if (list) list.push(ref);
      else backlinks.set(path, [ref]);
    }
    outgoing.set(doc.path, resolvedHere);
  }
  return { backlinks, outgoing, broken };
}
