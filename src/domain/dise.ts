import { parseFrontmatter, splitFrontmatter, type FrontmatterData } from "./frontmatter";
import {
  fencedCodeMask,
  lineCount,
  parseHeading,
  splitLines,
  trimBlankLines,
} from "./markdown";

/** One principle, `### D01 标题` in `self/底色.md`. */
export interface DiseEntry {
  id: string;
  /** Title without the check mark and without trailing notes such as （认同）. */
  title: string;
  /** Heading text after the id, without the check mark. */
  rawTitle: string;
  /** Trailing full-width parenthetical notes, e.g. ["认同"]; the draft marker is excluded. */
  notes: string[];
  /** The heading ends with ` ✅`: the user confirmed this entry. */
  confirmed: boolean;
  /** The heading carries the draft marker （草稿，待你确认）: the AI drafted it from the user's words. */
  draft: boolean;
  /** The user's own words: the first blockquote of the entry, without `>` markers. */
  quote: string;
  /** Everything in the entry after that blockquote (usually bullets). */
  bodyMarkdown: string;
  /** 0-based line index of the heading in the whole file. */
  line: number;
  sectionIndex: number;
}

export interface DiseSection {
  /** Full heading text, e.g. `一、怎么做选择`. */
  heading: string;
  /** Chinese ordinal before 、 (e.g. `一`), if present. */
  ordinal: string | null;
  title: string;
  /** Text between the section heading and its first entry. */
  introMarkdown: string;
  entries: DiseEntry[];
}

export interface DiseDoc {
  frontmatter: FrontmatterData;
  title: string | null;
  /** Text between the H1 and the first section (the AI's notes). */
  introMarkdown: string;
  sections: DiseSection[];
  entries: DiseEntry[];
}

const ENTRY_HEADING = /^(D\d+)(?![0-9A-Za-z])\s*(.*)$/;
const SECTION_ORDINAL = /^([一二三四五六七八九十百零〇]+)[、.．]\s*(.*)$/;
const CHECK = "✅";
/** Trailing check mark, tolerating an emoji variation selector. */
const TRAILING_CHECK = /[ \t]*✅\uFE0F?$/;
const TRAILING_NOTE = /[（(]([^（）()]*)[）)]\s*$/;
/**
 * The marker of an entry the AI drafted: （草稿，待你确认）, also with ASCII
 * parentheses or comma and stray spaces, together with the spaces before it.
 */
const DRAFT_MARKER_SOURCE = String.raw`[ \t]*[（(][ \t]*草稿[ \t]*[，,][ \t]*待你确认[ \t]*[）)]`;
const DRAFT_MARKER = new RegExp(DRAFT_MARKER_SOURCE);
const DRAFT_MARKERS = new RegExp(DRAFT_MARKER_SOURCE, "g");
/** The marker as AGENTS.md writes it. */
export const DRAFT_MARKER_TEXT = "（草稿，待你确认）";

/** True when a heading (or any text) carries the draft marker. */
export function hasDraftMarker(text: string): boolean {
  return DRAFT_MARKER.test(text);
}

/** `text` without any draft marker (and the spaces before it). */
function withoutDraftMarker(text: string): string {
  return text.replace(DRAFT_MARKERS, "");
}

interface TitleParts {
  title: string;
  rawTitle: string;
  notes: string[];
  confirmed: boolean;
  draft: boolean;
}

function parseEntryTitle(rest: string): TitleParts {
  let text = rest.trim();
  const confirmed = TRAILING_CHECK.test(text);
  if (confirmed) text = text.replace(TRAILING_CHECK, "").trimEnd();
  const rawTitle = text;
  const draft = hasDraftMarker(text);
  if (draft) text = withoutDraftMarker(text).trim();

  const notes: string[] = [];
  for (;;) {
    const match = TRAILING_NOTE.exec(text);
    if (!match || match.index === 0) break;
    const note = (match[1] ?? "").trim();
    if (note !== "") notes.unshift(note);
    text = text.slice(0, match.index).trimEnd();
  }
  return { title: text, rawTitle, notes, confirmed, draft };
}

/** Splits an entry's lines into the leading blockquote and the rest. */
function splitQuote(lines: string[]): { quote: string; body: string } {
  let i = 0;
  while (i < lines.length && (lines[i] ?? "").trim() === "") i += 1;
  const quoteLines: string[] = [];
  while (i < lines.length && /^\s{0,3}>/.test(lines[i] ?? "")) {
    quoteLines.push((lines[i] ?? "").replace(/\r$/, "").replace(/^\s{0,3}>\s?/, ""));
    i += 1;
  }
  return {
    quote: trimBlankLines(quoteLines),
    body: trimBlankLines(lines.slice(i)),
  };
}

/** Parses `self/底色.md` into sections and entries. */
export function parseDise(markdown: string): DiseDoc {
  const { data, block, body } = parseFrontmatter(markdown);
  const offset = lineCount(block);
  const lines = splitLines(body);
  const mask = fencedCodeMask(lines);

  let title: string | null = null;
  const intro: string[] = [];
  const sections: DiseSection[] = [];
  const entries: DiseEntry[] = [];

  type OpenSection = { heading: string; intro: string[] };
  type OpenEntry = { id: string; parts: TitleParts; line: number; lines: string[] };
  let section: OpenSection | null = null;
  let entry: OpenEntry | null = null;
  let sectionEntries: DiseEntry[] = [];

  const closeEntry = () => {
    if (!entry) return;
    const { quote, body: entryBody } = splitQuote(entry.lines);
    const parsed: DiseEntry = {
      id: entry.id,
      title: entry.parts.title,
      rawTitle: entry.parts.rawTitle,
      notes: entry.parts.notes,
      confirmed: entry.parts.confirmed,
      draft: entry.parts.draft,
      quote,
      bodyMarkdown: entryBody,
      line: entry.line,
      sectionIndex: Math.max(0, sections.length),
    };
    sectionEntries.push(parsed);
    entries.push(parsed);
    entry = null;
  };

  const closeSection = () => {
    closeEntry();
    const open = section as OpenSection | null;
    // Entries that appear before any `## ` heading get an untitled section.
    if (!open && sectionEntries.length === 0) return;
    const heading = open?.heading ?? "";
    const ordinal = SECTION_ORDINAL.exec(heading);
    sections.push({
      heading,
      ordinal: ordinal ? (ordinal[1] ?? null) : null,
      title: ordinal ? (ordinal[2] ?? "").trim() : heading,
      introMarkdown: trimBlankLines(open?.intro ?? []),
      entries: sectionEntries,
    });
    section = null;
    sectionEntries = [];
  };

  lines.forEach((line, index) => {
    const heading = mask[index] ? null : parseHeading(line);
    if (heading?.level === 1 && title === null && !section) {
      title = heading.text;
      return;
    }
    if (heading?.level === 2) {
      closeSection();
      section = { heading: heading.text, intro: [] };
      return;
    }
    if (heading?.level === 3) {
      const match = ENTRY_HEADING.exec(heading.text);
      if (match) {
        closeEntry();
        entry = {
          id: match[1] ?? "",
          parts: parseEntryTitle(match[2] ?? ""),
          line: offset + index,
          lines: [],
        };
        return;
      }
    }
    if (entry) entry.lines.push(line);
    else if (section) section.intro.push(line);
    else intro.push(line);
  });
  closeSection();

  return {
    frontmatter: data,
    title,
    introMarkdown: trimBlankLines(intro),
    sections,
    entries,
  };
}

/**
 * Puts entry `id` into the requested state: `desired` true ends its heading
 * with ` ✅`, false removes the check mark. Intent-based on purpose: when the
 * entry is already in that state the input is returned unchanged, so a stale
 * screen can never flip a confirmation the user did not mean to change.
 *
 * Confirming a draft also removes its marker （草稿，待你确认）, in the same
 * edit of the same line: `### D04 标题（草稿，待你确认）` becomes
 * `### D04 标题 ✅`. Un-confirming only ever removes the check mark; it never
 * puts a marker back.
 *
 * Only the heading line changes; trailing whitespace and CR are kept, so for
 * an entry that is not a draft, setting the old state again restores the
 * input exactly. Headings inside fenced code and the frontmatter are never
 * touched. Throws when the entry does not exist.
 */
export function setConfirmed(markdown: string, id: string, desired: boolean): string {
  const lines = splitLines(markdown);
  const offset = lineCount(splitFrontmatter(markdown).block);
  const mask = fencedCodeMask(lines.slice(offset));
  const pattern = new RegExp(`^ {0,3}###[ \\t]+${escapeRegExp(id)}(?![0-9A-Za-z])`);

  for (let i = offset; i < lines.length; i += 1) {
    const line = lines[i] ?? "";
    if (mask[i - offset] || !pattern.test(line)) continue;
    const trailing = /\s*$/.exec(line)?.[0] ?? "";
    const content = line.slice(0, line.length - trailing.length);
    const confirmed = TRAILING_CHECK.test(content);
    if (confirmed === desired) return markdown;
    lines[i] =
      (desired ? `${withoutDraftMarker(content).trimEnd()} ${CHECK}` : content.replace(TRAILING_CHECK, "")) + trailing;
    return lines.join("\n");
  }
  throw new Error(`底色里没有找到条目 ${id}`);
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** The exact source text of one entry, as {@link diseEntryTexts} returns it. */
export interface DiseEntryText {
  id: string;
  confirmed: boolean;
  /**
   * From the heading line up to the next entry or section heading, byte for
   * byte (CR and trailing spaces kept), without the blank lines at its end.
   */
  text: string;
}

/**
 * The source text of every entry of `self/底色.md`, in document order. Used to
 * prove that confirmed (✅) entries were not touched: blank lines between
 * entries are not part of an entry, so adding an entry after a confirmed one
 * leaves its text identical. Entry boundaries are the ones {@link parseDise}
 * uses; headings in fenced code and the frontmatter are not boundaries.
 */
export function diseEntryTexts(markdown: string): DiseEntryText[] {
  const lines = splitLines(markdown);
  const offset = lineCount(splitFrontmatter(markdown).block);
  const mask = fencedCodeMask(lines.slice(offset));
  const entries = parseDise(markdown).entries;
  const starts = new Set(entries.map((entry) => entry.line));

  return entries.map((entry) => {
    let end = entry.line + 1;
    while (end < lines.length) {
      if (starts.has(end)) break;
      const heading = mask[end - offset] ? null : parseHeading(lines[end] ?? "");
      if (heading && heading.level <= 2) break;
      end += 1;
    }
    while (end > entry.line + 1 && (lines[end - 1] ?? "").trim() === "") end -= 1;
    return {
      id: entry.id,
      confirmed: entry.confirmed,
      text: lines.slice(entry.line, end).join("\n"),
    };
  });
}

/** Picks one item per local calendar day; consecutive days walk through the list. */
export function pickDailyEntry<T>(items: readonly T[], date: Date): T | null {
  if (items.length === 0) return null;
  const day = Math.floor(
    Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / 86_400_000,
  );
  const index = ((day % items.length) + items.length) % items.length;
  return items[index] ?? null;
}
