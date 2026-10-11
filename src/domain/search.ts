import { parseWikilinkInner } from "./wikilinks";

/** Anything with a title and searchable text. */
export interface Searchable {
  path: string;
  title: string;
  name: string;
  content: string;
}

export interface SearchHit<T extends Searchable = Searchable> {
  doc: T;
  titleMatch: boolean;
  /** Plain-text excerpt around the first body match, or null. */
  snippet: string | null;
  score: number;
}

/** Turns Markdown into plain words for searching and excerpts. */
export function stripMarkdown(markdown: string): string {
  return markdown
    .replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, "")
    .replace(/!?\[\[([^[\]\n]+?)\]\]/g, (_m, inner: string) => {
      const { target, heading, alias } = parseWikilinkInner(inner);
      return alias ?? (target || heading || "");
    })
    .replace(/!?\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/^\s{0,3}#{1,6}\s+/gm, "")
    .replace(/^\s{0,3}>\s?/gm, "")
    .replace(/^\s*[-*+]\s+\[[ xX]\]\s+/gm, "")
    .replace(/^\s*(?:[-*+]|\d+[.)])\s+/gm, "")
    .replace(/^\s*```.*$/gm, "")
    .replace(/[*_~`|]+/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

/** A window of `radius` characters on each side of the first matching term. */
export function makeSnippet(
  text: string,
  terms: readonly string[],
  radius = 36,
): string | null {
  const lower = text.toLowerCase();
  let best = -1;
  let length = 0;
  for (const term of terms) {
    const index = lower.indexOf(term.toLowerCase());
    if (index !== -1 && (best === -1 || index < best)) {
      best = index;
      length = term.length;
    }
  }
  if (best === -1) return null;
  const start = Math.max(0, best - radius);
  const end = Math.min(text.length, best + length + radius);
  return `${start > 0 ? "…" : ""}${text.slice(start, end)}${end < text.length ? "…" : ""}`;
}

/** Splits a query into lower-case terms. */
export function queryTerms(query: string): string[] {
  return query
    .toLowerCase()
    .split(/\s+/)
    .map((term) => term.trim())
    .filter(Boolean);
}

/**
 * Full-text search: every term must appear in the title, the file name or the
 * text. Title matches rank first, then by number of occurrences.
 */
export function searchDocs<T extends Searchable>(
  docs: readonly T[],
  query: string,
): SearchHit<T>[] {
  const terms = queryTerms(query);
  if (terms.length === 0) return [];

  const hits: SearchHit<T>[] = [];
  for (const doc of docs) {
    const title = `${doc.title} ${doc.name}`.toLowerCase();
    const plain = stripMarkdown(doc.content);
    const text = plain.toLowerCase();
    if (!terms.every((term) => title.includes(term) || text.includes(term))) continue;

    const titleMatch = terms.every((term) => title.includes(term));
    const occurrences = terms.reduce((sum, term) => sum + countOccurrences(text, term), 0);
    hits.push({
      doc,
      titleMatch,
      snippet: makeSnippet(plain, terms),
      score: (titleMatch ? 1000 : 0) + occurrences,
    });
  }
  return hits.sort(
    (a, b) => b.score - a.score || (a.doc.path < b.doc.path ? -1 : a.doc.path > b.doc.path ? 1 : 0),
  );
}

function countOccurrences(text: string, term: string): number {
  let count = 0;
  for (let i = text.indexOf(term); i !== -1; i = text.indexOf(term, i + term.length)) {
    count += 1;
  }
  return count;
}
