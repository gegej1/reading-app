import { describe, expect, it } from "vitest";
import { parsePrecedent, precedentRulingTexts } from "./precedents";
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

describe("precedentRulingTexts", () => {
  const P004 = [
    "---",
    "类型: 判例",
    "编号: P-004",
    "说明: |",
    "  ## 裁决",
    "  frontmatter 里的不是标题",
    "---",
    "",
    "# P-004 工具与习惯",
    "",
    "## 冲突",
    "- 统一：省沟通成本",
    "",
    "## 裁决",
    "各组先用自己顺手的工具。",
    "",
    "> 工具是给人用的，不是给表格用的。",
    "",
    "### 补充",
    "三个月后再看。",
    "",
    "```text",
    "## 代码块里的标题不算",
    "```",
    "",
    "## 适用规则（AI 整理，待你确认）",
    "1. 先问：换工具的成本谁来出？",
    "",
  ].join("\n");

  it("returns the body of the 裁决 section byte for byte, up to the next section", () => {
    expect(precedentRulingTexts(P004)).toEqual([
      [
        "各组先用自己顺手的工具。",
        "",
        "> 工具是给人用的，不是给表格用的。",
        "",
        "### 补充",
        "三个月后再看。",
        "",
        "```text",
        "## 代码块里的标题不算",
        "```",
      ].join("\n"),
    ]);
  });

  it("drops blank lines around the body but keeps CR and trailing spaces inside it", () => {
    const text = "## 裁决\r\n\r\n第一句  \r\n第二句\r\n\r\n## 适用规则\r\n";
    expect(precedentRulingTexts(text)).toEqual(["第一句  \r\n第二句\r"]);
  });

  it("ends the last section at the end of the file and knows annotated headings", () => {
    expect(precedentRulingTexts("# P\n\n## 裁决（2026-10-11）\n原话\n")).toEqual(["原话"]);
    expect(precedentRulingTexts("## 裁决\n甲\n## 裁决\n乙\n")).toEqual(["甲", "乙"]);
  });

  it("finds nothing in a precedent without a 裁决 section", () => {
    expect(precedentRulingTexts("# P\n\n## 裁决依据\n不是裁决\n### 裁决\n也不是\n")).toEqual([]);
    expect(precedentRulingTexts(fixture("self/判例/P-001-速度与退路.md"))).toEqual([
      "选 ①：先确认退路，再决定速度。\n\n> 能退回来的事情我愿意快，退不回来的事情我宁可慢。",
    ]);
  });
});
