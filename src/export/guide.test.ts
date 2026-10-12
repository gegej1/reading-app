// The iPhone guide has one source, shortcuts/使用说明.md. The app puts
// shortcuts/使用说明.html (rendered from it in the phone pages' look) into the
// bridge folder, and the README carries the same text. This keeps all three
// in step: `pnpm guide` writes the HTML again.

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { renderGuide } from "./phonePages";

const root = process.cwd();
const read = (path: string) => readFileSync(join(root, path), "utf8");

describe("the iPhone guide", () => {
  const markdown = read("shortcuts/使用说明.md");
  const html = renderGuide(markdown);

  it("is checked in as HTML, rendered from its Markdown", () => {
    const file = join(root, "shortcuts/使用说明.html");
    if (process.env.DISE_UPDATE_GUIDE === "1") writeFileSync(file, html);
    expect(read("shortcuts/使用说明.html"), "run pnpm guide").toBe(html);
    expect(html).not.toMatch(/<script|\son[a-z]+=|\ssrc=/i);
    for (const name of ["记一笔", "看底色", "答裁决"]) expect(html).toContain(name);
  });

  it("is in the README word for word", () => {
    const readme = read("README.md");
    const start = readme.indexOf("<!-- iphone-guide:start -->");
    const end = readme.indexOf("<!-- iphone-guide:end -->");
    expect(start).toBeGreaterThan(-1);
    expect(end).toBeGreaterThan(start);
    const section = readme.slice(start + "<!-- iphone-guide:start -->".length, end).trim();
    expect(section).toBe(markdown.trim());
  });
});
