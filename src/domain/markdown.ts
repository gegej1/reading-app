// Small line-based Markdown helpers shared by the KB parsers.
// They only understand what the KB format needs: ATX headings and fenced code.

export interface Heading {
  level: number;
  text: string;
}

export interface Section {
  /** Heading text without the leading hashes. */
  heading: string;
  level: number;
  /** 0-based line index of the heading within the text that was split. */
  line: number;
  /** Section content without the heading, trimmed of blank lines at both ends. */
  markdown: string;
}

/** Splits on LF only, so `lines.join("\n")` restores the input exactly. */
export function splitLines(text: string): string[] {
  return text.split("\n");
}

const HEADING = /^ {0,3}(#{1,6})(?:[ \t]+(.*))?$/;

/** Parses an ATX heading line (`## 标题`), or returns null. */
export function parseHeading(line: string): Heading | null {
  const match = HEADING.exec(line.replace(/\r$/, ""));
  if (!match) {
    return null;
  }
  const hashes = match[1] ?? "";
  const text = (match[2] ?? "")
    .trim()
    .replace(/(?:^|[ \t]+)#+$/, "")
    .trim();
  return { level: hashes.length, text };
}

const FENCE_OPEN = /^[ \t]*(`{3,}|~{3,})(.*)$/;
const FENCE_CLOSE = /^[ \t]*(`{3,}|~{3,})[ \t]*$/;

/** Marks every line that belongs to a fenced code block, fences included. */
export function fencedCodeMask(lines: readonly string[]): boolean[] {
  const mask: boolean[] = [];
  let fence: { char: string; length: number } | null = null;

  for (const raw of lines) {
    const line = raw.replace(/\r$/, "");
    if (fence === null) {
      const open = FENCE_OPEN.exec(line);
      const marker = open?.[1] ?? "";
      const info = open?.[2] ?? "";
      // A backtick fence's info string may not contain backticks (CommonMark).
      if (open && !(marker.startsWith("`") && info.includes("`"))) {
        fence = { char: marker.charAt(0), length: marker.length };
        mask.push(true);
      } else {
        mask.push(false);
      }
      continue;
    }

    mask.push(true);
    const close = FENCE_CLOSE.exec(line);
    const marker = close?.[1] ?? "";
    if (close && marker.charAt(0) === fence.char && marker.length >= fence.length) {
      fence = null;
    }
  }
  return mask;
}

/** Drops whitespace-only lines from both ends and joins the rest. */
export function trimBlankLines(lines: readonly string[]): string {
  let start = 0;
  let end = lines.length;
  while (start < end && (lines[start] ?? "").trim() === "") start += 1;
  while (end > start && (lines[end - 1] ?? "").trim() === "") end -= 1;
  return lines.slice(start, end).join("\n");
}

/**
 * Splits text into sections at headings of exactly `level`. Deeper headings stay
 * inside a section; a shallower heading ends it. Text before the first section
 * becomes the preamble. Headings inside fenced code are ignored.
 */
export function splitSections(
  text: string,
  level: number,
): { preamble: string; sections: Section[] } {
  const lines = splitLines(text);
  const mask = fencedCodeMask(lines);
  const preamble: string[] = [];
  const sections: Section[] = [];
  let current: { heading: string; line: number; lines: string[] } | null = null;
  let started = false;

  const close = () => {
    if (current) {
      sections.push({
        heading: current.heading,
        level,
        line: current.line,
        markdown: trimBlankLines(current.lines),
      });
      current = null;
    }
  };

  lines.forEach((line, index) => {
    const heading = mask[index] ? null : parseHeading(line);
    if (heading && heading.level <= level) {
      close();
      if (heading.level === level) {
        started = true;
        current = { heading: heading.text, line: index, lines: [] };
        return;
      }
    }
    if (current) {
      current.lines.push(line);
    } else if (!started) {
      preamble.push(line);
    }
  });
  close();

  return { preamble: trimBlankLines(preamble), sections };
}

/** Returns the text of the first heading of `level` outside fenced code. */
export function firstHeading(text: string, level: number): string | null {
  const lines = splitLines(text);
  const mask = fencedCodeMask(lines);
  for (let i = 0; i < lines.length; i += 1) {
    if (mask[i]) continue;
    const heading = parseHeading(lines[i] ?? "");
    if (heading && heading.level === level) {
      return heading.text;
    }
  }
  return null;
}

/** Number of lines a frontmatter block occupies (0 when there is none). */
export function lineCount(block: string): number {
  if (block === "") return 0;
  return splitLines(block).length - (block.endsWith("\n") ? 1 : 0);
}
