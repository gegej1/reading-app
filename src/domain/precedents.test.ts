import { describe, expect, it } from "vitest";
import { parsePrecedent } from "./precedents";
import { fixture } from "./__fixtures__";

describe("parsePrecedent", () => {
  it("reads frontmatter fields, the title and sections", () => {
    const path = "self/判例/P-001-速度与退路.md";
    const p = parsePrecedent(path, fixture(path));
    expect(p).toMatchObject({
      path,
      name: "P-001-速度与退路",
      id: "P-001",
      title: "速度与退路：先冲还是先稳",
      shortTitle: "速度与退路",
      status: "生效（适用规则待你确认）",
      rulingDate: "2026-09-01",
      related: ["D01"],
    });
    expect(p.sections.map((s) => s.heading)).toEqual([
      "冲突",
      "裁决",
      "适用规则（AI 整理，待你确认）",
      "原话",
    ]);
    expect(p.sections[1]?.markdown).toContain("> 能退回来的事情我愿意快");
  });

  it("splits multi-valued relations", () => {
    const path = "self/判例/P-002-完整与及时.md";
    expect(parsePrecedent(path, fixture(path)).related).toEqual(["D04", "D02"]);
  });

  it("falls back to the file name when frontmatter and H1 are missing", () => {
    const p = parsePrecedent("self/判例/P-009-临时.md", "只有正文\n");
    expect(p).toMatchObject({
      id: "P-009",
      title: "临时",
      shortTitle: "临时",
      status: null,
      rulingDate: null,
      related: [],
    });
  });
});
