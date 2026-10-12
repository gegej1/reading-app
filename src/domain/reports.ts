import { parseFrontmatter, splitFrontmatter, type FrontmatterData } from "./frontmatter";
import {
  fencedCodeMask,
  firstHeading,
  lineCount,
  parseHeading,
  splitLines,
} from "./markdown";
import { basenameOf, parseWikilinkInner } from "./wikilinks";

/** One line of the 待你抽查 checklist: an AI ruling that the user audits. */
export interface AuditItem {
  /** Raw item id such as `I-12`, or null when the line has none. */
  itemId: string | null;
  checked: boolean;
  /** Line text after the checkbox. */
  text: string;
  /** Short summary of the new material (before ↔). */
  summary: string;
  /** What the material is in tension with (between ↔ and ：). */
  tension: string | null;
  /** The AI's ruling (after ：, without the 依据 note). */
  ruling: string | null;
  /** Name of the precedent the AI relied on, e.g. `P-001-速度与退路`. */
  precedent: string | null;
  /** 0-based line index in the whole report file. */
  line: number;
  reportPath: string;
}

export type AuditLineParts = Pick<
  AuditItem,
  "itemId" | "summary" | "tension" | "ruling" | "precedent"
>;

export interface Report {
  path: string;
  name: string;
  /** `YYYY-MM-DD` from the file name. */
  date: string | null;
  /** Same-day sequence: 1 for the first report, 2 for a `-2` suffix, … */
  sequence: number;
  title: string | null;
  /** The text after `结果：`. */
  resultLine: string | null;
  resultParts: string[];
  auditItems: AuditItem[];
  frontmatter: FrontmatterData;
  body: string;
}

const ITEM_ID = /^(I-\d+)(?![0-9A-Za-z])\s*/;
const BASIS = /\s*[（(]\s*依据\s*(.*?)\s*[）)]\s*$/;
const CHECKBOX = /^(\s*[-*+]\s+\[)([ xX])(\]\s+)(.*)$/;
const RESULT_LINE = /^\s*[-*+]\s*结果\s*[：:]\s*(.+?)\s*$/;

/** Parses `I-12 摘要 ↔ 张力：裁决（依据 [[P-001-…]]）`. Missing parts become null. */
export function parseAuditLine(text: string): AuditLineParts {
  let rest = text.trim();
  const id = ITEM_ID.exec(rest);
  const itemId = id ? (id[1] ?? null) : null;
  if (id) rest = rest.slice(id[0].length);

  let precedent: string | null = null;
  const basis = BASIS.exec(rest);
  if (basis) {
    const link = /\[\[([^\]]+)\]\]/.exec(basis[1] ?? "");
    precedent = link ? parseWikilinkInner(link[1] ?? "").target || null : null;
    rest = rest.slice(0, basis.index);
  }

  const arrow = rest.indexOf("↔");
  if (arrow === -1) {
    return { itemId, summary: rest.trim(), tension: null, ruling: null, precedent };
  }
  const summary = rest.slice(0, arrow).trim();
  const after = rest.slice(arrow + 1).trim();
  const colon = after.search(/[：:]/);
  const tension = (colon === -1 ? after : after.slice(0, colon)).trim();
  const ruling = colon === -1 ? "" : after.slice(colon + 1).trim();
  return {
    itemId,
    summary,
    tension: tension === "" ? null : tension,
    ruling: ruling === "" ? null : ruling,
    precedent,
  };
}

/** Date and same-day sequence from a report file name. */
export function reportDateFromPath(path: string): { date: string | null; sequence: number } {
  const name = basenameOf(path);
  const date = /^(\d{4}-\d{2}-\d{2})/.exec(name)?.[1] ?? null;
  const suffix = date ? /-(\d+)$/.exec(name.slice(date.length)) : null;
  return { date, sequence: suffix ? Number(suffix[1]) : 1 };
}

/** Line ranges [start, end) of `## 待你抽查…` sections, in body coordinates. */
function auditRanges(lines: string[], mask: boolean[]): [number, number][] {
  const ranges: [number, number][] = [];
  let start = -1;
  lines.forEach((line, index) => {
    const heading = mask[index] ? null : parseHeading(line);
    if (!heading || heading.level > 2) return;
    if (start !== -1) {
      ranges.push([start, index]);
      start = -1;
    }
    if (heading.level === 2 && heading.text.startsWith("待你抽查")) start = index + 1;
  });
  if (start !== -1) ranges.push([start, lines.length]);
  return ranges;
}

/** A piece of a report body: Markdown, or the place of a 待你抽查 checklist. */
export type ReportSegment =
  | { kind: "markdown"; markdown: string }
  | {
      kind: "audit";
      /** The section heading (without `## `). */
      heading: string;
      /** What else the section holds besides the checklist lines. */
      rest: string;
    };

/**
 * Cuts a report body (without frontmatter) at its 待你抽查 sections, so a
 * page can show the checklist as controls and the rest as Markdown. The
 * checklist lines themselves are left out of the segments; joined, the
 * Markdown segments, headings and rests hold every other line.
 */
export function reportSegments(body: string): ReportSegment[] {
  const lines = splitLines(body);
  const mask = fencedCodeMask(lines);
  const segments: ReportSegment[] = [];
  let from = 0;
  for (const [start, end] of auditRanges(lines, mask)) {
    const headingLine = start - 1;
    const before = lines.slice(from, headingLine).join("\n");
    if (before.trim() !== "") segments.push({ kind: "markdown", markdown: before });
    const rest = lines
      .slice(start, end)
      .filter((line, offset) => mask[start + offset] || !CHECKBOX.test(line.replace(/\r$/, "")))
      .join("\n");
    segments.push({ kind: "audit", heading: parseHeading(lines[headingLine] ?? "")?.text ?? "", rest: rest.trim() });
    from = end;
  }
  const after = lines.slice(from).join("\n");
  if (after.trim() !== "") segments.push({ kind: "markdown", markdown: after });
  return segments;
}

/** Parses an ingest report. */
export function parseReport(path: string, markdown: string): Report {
  const { data, block, body } = parseFrontmatter(markdown);
  const offset = lineCount(block);
  const lines = splitLines(body);
  const mask = fencedCodeMask(lines);

  let resultLine: string | null = null;
  for (let i = 0; i < lines.length && resultLine === null; i += 1) {
    if (mask[i]) continue;
    const match = RESULT_LINE.exec((lines[i] ?? "").replace(/\r$/, ""));
    if (match) resultLine = match[1] ?? null;
  }

  const auditItems: AuditItem[] = [];
  for (const [start, end] of auditRanges(lines, mask)) {
    for (let i = start; i < end; i += 1) {
      if (mask[i]) continue;
      const match = CHECKBOX.exec((lines[i] ?? "").replace(/\r$/, ""));
      if (!match) continue;
      const text = (match[4] ?? "").trim();
      auditItems.push({
        ...parseAuditLine(text),
        checked: match[2] !== " ",
        text,
        line: offset + i,
        reportPath: path,
      });
    }
  }

  const { date, sequence } = reportDateFromPath(path);
  const parts =
    resultLine === null
      ? []
      : resultLine
          .split(/[｜|]/)
          .map((part) => part.trim())
          .filter(Boolean);
  return {
    path,
    name: basenameOf(path),
    date,
    sequence,
    title: firstHeading(body, 1),
    resultLine,
    resultParts: parts,
    auditItems,
    frontmatter: data,
    body,
  };
}

/** Newest first: by date, then by same-day sequence, then by path. Undated last. */
export function sortReportsNewestFirst(reports: readonly Report[]): Report[] {
  return [...reports].sort((a, b) => {
    if (a.date !== b.date) {
      if (a.date === null) return 1;
      if (b.date === null) return -1;
      return a.date < b.date ? 1 : -1;
    }
    if (a.sequence !== b.sequence) return b.sequence - a.sequence;
    return a.path < b.path ? 1 : a.path > b.path ? -1 : 0;
  });
}

/**
 * Puts one 待你抽查 item into the requested state: `desired` true checks it
 * (`- [x]`), false unchecks it (`- [ ]`). When the item is already in that
 * state the input is returned unchanged, so a stale screen never flips an
 * audit the user did not mean to change. Only that line changes. Throws when
 * the item is not in an audit section.
 */
export function setChecked(markdown: string, itemId: string, desired: boolean): string {
  const lines = splitLines(markdown);
  // Work on the whole file so frontmatter lines are never candidates: audit
  // sections only start at a `## 待你抽查` heading in the body.
  const offset = lineCount(splitFrontmatter(markdown).block);
  const bodyLines = lines.slice(offset);
  const bodyMask = fencedCodeMask(bodyLines);

  for (const [start, end] of auditRanges(bodyLines, bodyMask)) {
    for (let i = start; i < end; i += 1) {
      if (bodyMask[i]) continue;
      const line = bodyLines[i] ?? "";
      const cr = line.endsWith("\r") ? "\r" : "";
      const match = CHECKBOX.exec(cr ? line.slice(0, -1) : line);
      if (!match) continue;
      const id = ITEM_ID.exec((match[4] ?? "").trim());
      if (id?.[1] !== itemId) continue;
      const checked = match[2] !== " ";
      if (checked === desired) return markdown;
      lines[offset + i] = `${match[1]}${desired ? "x" : " "}${match[3]}${match[4]}${cr}`;
      return lines.join("\n");
    }
  }
  throw new Error(`入库报告的待抽查清单里没有 ${itemId}`);
}
