import { fmList, fmString, parseFrontmatter, splitFrontmatter, type FrontmatterData } from "./frontmatter";
import { fencedCodeMask, firstHeading, lineCount, parseHeading, splitLines, splitSections } from "./markdown";
import { basenameOf } from "./wikilinks";

export interface DocSection {
  heading: string;
  markdown: string;
}

/** A precedent file `self/判例/P-001-标题.md`. */
export interface Precedent {
  path: string;
  /** File basename, the name used by wikilinks. */
  name: string;
  id: string;
  /** H1 without the id, e.g. `速度与退路：先冲还是先稳`. */
  title: string;
  /** Title from the file name, e.g. `速度与退路`. */
  shortTitle: string;
  status: string | null;
  /** Date of the ruling (`裁决` field). */
  rulingDate: string | null;
  /** Related principles (`关联` field), e.g. ["D05", "D04"]. */
  related: string[];
  frontmatter: FrontmatterData;
  sections: DocSection[];
  body: string;
}

export interface IdentifiedTitle {
  id: string;
  title: string;
  shortTitle: string;
}

/**
 * Derives id and titles for numbered KB pages (`P-001-…`, `Q-001-…`): the id comes
 * from frontmatter `编号`, then from the file name; the title from the H1 with the
 * id prefix removed, falling back to the file name.
 */
export function identify(
  path: string,
  frontmatterId: string | null,
  body: string,
  idPattern: RegExp,
): IdentifiedTitle {
  const name = basenameOf(path);
  const fromName = idPattern.exec(name);
  const id = frontmatterId ?? fromName?.[0] ?? name;
  const shortTitle =
    fromName && name.startsWith(fromName[0])
      ? name.slice(fromName[0].length).replace(/^[-_\s]+/, "") || name
      : name;
  const h1 = firstHeading(body, 1);
  const title = h1 ? stripIdPrefix(h1, id) : shortTitle;
  return { id, title: title === "" ? shortTitle : title, shortTitle };
}

function stripIdPrefix(heading: string, id: string): string {
  return heading.startsWith(id) ? heading.slice(id.length).trim() : heading.trim();
}

/** Splits a body into its `## ` sections. */
export function docSections(body: string): DocSection[] {
  return splitSections(body, 2).sections.map(({ heading, markdown }) => ({
    heading,
    markdown,
  }));
}

/** Parses one precedent file. */
export function parsePrecedent(path: string, markdown: string): Precedent {
  const { data, body } = parseFrontmatter(markdown);
  const { id, title, shortTitle } = identify(
    path,
    fmString(data, "编号"),
    body,
    /^P-\d+/,
  );
  return {
    path,
    name: basenameOf(path),
    id,
    title,
    shortTitle,
    status: fmString(data, "状态"),
    rulingDate: fmString(data, "裁决"),
    related: fmList(data, "关联"),
    frontmatter: data,
    sections: docSections(body),
    body,
  };
}

/**
 * A `## 裁决` heading: the section of a precedent that holds the user's own
 * words. A trailing note in parentheses is allowed (`## 裁决（2026-10-11）`).
 */
const RULING_HEADING = /^裁决(?:\s*[（(][^（）()]*[）)])?$/;

/**
 * The body of every `## 裁决` section of a precedent, in document order: the
 * lines after the heading up to the next heading of level 1 or 2, byte for
 * byte (CR and trailing spaces kept), without the blank lines at either end.
 * Deeper headings and fenced code stay inside the body; headings inside
 * fenced code and the frontmatter are not headings. Used by the gate to prove
 * that a run left the user's words in existing precedents untouched.
 */
export function precedentRulingTexts(markdown: string): string[] {
  const lines = splitLines(markdown);
  const body = lines.slice(lineCount(splitFrontmatter(markdown).block));
  const mask = fencedCodeMask(body);
  const texts: string[] = [];
  let start = -1;

  const close = (end: number) => {
    let first = start;
    let last = end;
    while (first < last && (body[first] ?? "").trim() === "") first += 1;
    while (last > first && (body[last - 1] ?? "").trim() === "") last -= 1;
    texts.push(body.slice(first, last).join("\n"));
    start = -1;
  };

  body.forEach((line, index) => {
    const heading = mask[index] ? null : parseHeading(line);
    if (!heading || heading.level > 2) return;
    if (start !== -1) close(index);
    if (heading.level === 2 && RULING_HEADING.test(heading.text)) start = index + 1;
  });
  if (start !== -1) close(body.length);
  return texts;
}
