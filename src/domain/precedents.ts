import { fmList, fmString, parseFrontmatter, type FrontmatterData } from "./frontmatter";
import { firstHeading, splitSections } from "./markdown";
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
