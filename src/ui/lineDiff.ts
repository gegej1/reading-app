// A small line diff for the run panel's collation view (校勘): Myers'
// O((N+M)·D) algorithm on lines, unified hunks with context, and the changed
// part inside a line that was replaced by a similar one (character by
// character, so a single changed 汉字 stands out). No library: the files of
// a knowledge base are small and so are the changes of one run.

export type DiffKind = "same" | "add" | "del";

export interface DiffLine {
  kind: DiffKind;
  text: string;
  /** 1-based line numbers on each side; null where the line does not exist. */
  oldNo: number | null;
  newNo: number | null;
  /** The changed part of a replaced line, as [start, end) in code points. */
  mark: [number, number] | null;
}

export type DiffBlock = { kind: "lines"; lines: DiffLine[] } | { kind: "gap"; lines: DiffLine[] };

export interface FileDiff {
  lines: DiffLine[];
  added: number;
  removed: number;
  /** One side ends with a newline and the other does not. */
  endNewlineChanged: boolean;
  /** The files were too different to compare line by line: shown as replaced. */
  tooLarge: boolean;
}

/** More edits than this and the files are shown as replaced (memory grows with its square). */
const MAX_EDITS = 2000;

function linesOf(text: string): string[] {
  if (text === "") return [];
  const lines = text.split("\n");
  if (lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/** The shortest edit script from `a` to `b` (Myers), as a list of kinds. */
function editScript(a: readonly string[], b: readonly string[]): DiffKind[] | null {
  const n = a.length;
  const m = b.length;
  const max = n + m;
  const offset = max + 1;
  const v = new Int32Array(2 * max + 3);
  // trace[d] holds V as it was before step d, for the diagonals -d … d only.
  const trace: Int32Array[] = [];
  let found = -1;
  for (let d = 0; d <= Math.min(max, MAX_EDITS); d += 1) {
    trace.push(v.slice(offset - d, offset + d + 1));
    for (let k = -d; k <= d; k += 2) {
      let x =
        k === -d || (k !== d && (v[offset + k - 1] ?? 0) < (v[offset + k + 1] ?? 0))
          ? (v[offset + k + 1] ?? 0)
          : (v[offset + k - 1] ?? 0) + 1;
      let y = x - k;
      while (x < n && y < m && a[x] === b[y]) {
        x += 1;
        y += 1;
      }
      v[offset + k] = x;
      if (x >= n && y >= m) {
        found = d;
        break;
      }
    }
    if (found !== -1) break;
  }
  if (found === -1) return null;

  // Walk back through the snapshots to recover the path.
  const kinds: DiffKind[] = [];
  let x = n;
  let y = m;
  for (let d = found; d > 0; d -= 1) {
    const snapshot = trace[d];
    if (!snapshot) return null;
    const at = (diagonal: number) => snapshot[diagonal + d] ?? 0;
    const k = x - y;
    const down = k === -d || (k !== d && at(k - 1) < at(k + 1));
    const prevK = down ? k + 1 : k - 1;
    const prevX = at(prevK);
    const prevY = prevX - prevK;
    while (x > prevX && y > prevY) {
      kinds.push("same");
      x -= 1;
      y -= 1;
    }
    kinds.push(down ? "add" : "del");
    x = prevX;
    y = prevY;
  }
  while (x > 0 && y > 0) {
    kinds.push("same");
    x -= 1;
    y -= 1;
  }
  return kinds.reverse();
}

/** The changed middle of `from` → `to`, in code points of each, or null when nothing is shared. */
function changedMiddle(from: string, to: string): { del: [number, number]; add: [number, number] } | null {
  const a = Array.from(from);
  const b = Array.from(to);
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start += 1;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA -= 1;
    endB -= 1;
  }
  const shared = start + (a.length - endA);
  // Lines that share almost nothing are simply different lines.
  if (shared === 0 || shared * 3 < Math.min(a.length, b.length)) return null;
  return { del: [start, endA], add: [start, endB] };
}

/** Marks the changed part of each removed line followed by an added one. */
function markReplacements(lines: DiffLine[]): void {
  let i = 0;
  while (i < lines.length) {
    if (lines[i]?.kind !== "del") {
      i += 1;
      continue;
    }
    let dels = i;
    while (dels < lines.length && lines[dels]?.kind === "del") dels += 1;
    let adds = dels;
    while (adds < lines.length && lines[adds]?.kind === "add") adds += 1;
    const pairs = Math.min(dels - i, adds - dels);
    for (let p = 0; p < pairs; p += 1) {
      const removed = lines[i + p];
      const added = lines[dels + p];
      if (!removed || !added) continue;
      const middle = changedMiddle(removed.text, added.text);
      if (middle) {
        removed.mark = middle.del;
        added.mark = middle.add;
      }
    }
    i = Math.max(adds, i + 1);
  }
}

/** Compares two versions of a file; null means the file does not exist on that side. */
export function diffFiles(before: string | null, after: string | null): FileDiff {
  const a = linesOf(before ?? "");
  const b = linesOf(after ?? "");
  const kinds = editScript(a, b);
  const lines: DiffLine[] = [];
  let oldNo = 0;
  let newNo = 0;
  const push = (kind: DiffKind) => {
    if (kind === "same") {
      oldNo += 1;
      newNo += 1;
      lines.push({ kind, text: b[newNo - 1] ?? "", oldNo, newNo, mark: null });
    } else if (kind === "del") {
      oldNo += 1;
      lines.push({ kind, text: a[oldNo - 1] ?? "", oldNo, newNo: null, mark: null });
    } else {
      newNo += 1;
      lines.push({ kind, text: b[newNo - 1] ?? "", oldNo: null, newNo, mark: null });
    }
  };
  if (kinds) {
    kinds.forEach(push);
    markReplacements(lines);
  } else {
    a.forEach(() => push("del"));
    b.forEach(() => push("add"));
  }
  const endsWithNewline = (text: string | null) => text !== null && text !== "" && text.endsWith("\n");
  return {
    lines,
    added: lines.filter((line) => line.kind === "add").length,
    removed: lines.filter((line) => line.kind === "del").length,
    endNewlineChanged:
      before !== null && after !== null && before !== "" && after !== "" && endsWithNewline(before) !== endsWithNewline(after),
    tooLarge: kinds === null,
  };
}

/**
 * Unified hunks: every changed line with `context` unchanged lines around
 * it; longer unchanged runs become a gap that can be opened.
 */
export function diffBlocks(lines: readonly DiffLine[], context = 3): DiffBlock[] {
  const keep = lines.map(() => false);
  lines.forEach((line, index) => {
    if (line.kind === "same") return;
    for (let i = Math.max(0, index - context); i <= Math.min(lines.length - 1, index + context); i += 1) keep[i] = true;
  });
  const blocks: DiffBlock[] = [];
  lines.forEach((line, index) => {
    const kind = keep[index] ? "lines" : "gap";
    const last = blocks[blocks.length - 1];
    if (last && last.kind === kind) last.lines.push(line);
    else blocks.push({ kind, lines: [line] });
  });
  return blocks;
}

/** Splits a line around its mark, for rendering. */
export function splitMark(line: Pick<DiffLine, "text" | "mark">): { before: string; marked: string; after: string } {
  if (!line.mark) return { before: line.text, marked: "", after: "" };
  const chars = Array.from(line.text);
  const [start, end] = line.mark;
  return {
    before: chars.slice(0, start).join(""),
    marked: chars.slice(start, end).join(""),
    after: chars.slice(end).join(""),
  };
}
