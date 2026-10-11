import { fencedCodeMask, parseHeading, splitLines } from "./markdown";
import { stripMarkdown } from "./search";

const ONE_LINER = /^\s*\*\*一句话\*\*\s*[：:]\s*(.+?)\s*$/;

/** The `**一句话**：…` summary line of a knowledge page, as plain text. */
export function oneLiner(body: string): string | null {
  for (const line of splitLines(body)) {
    const match = ONE_LINER.exec(line.replace(/\r$/, ""));
    if (match) return stripMarkdown(match[1] ?? "") || null;
  }
  return null;
}

/** Number of `## I-01` style items in a raw source file. */
export function countRawItems(body: string): number {
  const lines = splitLines(body);
  const mask = fencedCodeMask(lines);
  let count = 0;
  lines.forEach((line, index) => {
    if (mask[index]) return;
    const heading = parseHeading(line);
    if (heading?.level === 2 && /^I-\d+\b/.test(heading.text)) count += 1;
  });
  return count;
}

/**
 * Removes the page's H1 when it is the first thing in the body, so a page
 * view can show the title in its own header without repeating it.
 */
export function stripLeadingH1(body: string): string {
  const lines = splitLines(body);
  const first = lines.findIndex((line) => line.trim() !== "");
  if (first === -1 || parseHeading(lines[first] ?? "")?.level !== 1) return body;
  lines.splice(first, 1);
  return lines.join("\n");
}
