import { describe, expect, it } from "vitest";
import {
  analyzeLinks,
  basenameOf,
  buildLinkIndex,
  extractWikilinks,
  maskInlineCode,
  parseWikilinkInner,
} from "./wikilinks";
import { fixtureKb } from "./__fixtures__";

describe("parseWikilinkInner", () => {
  it("parses plain, aliased, heading and combined forms", () => {
    expect(parseWikilinkInner("示例概念")).toEqual({
      target: "示例概念",
      heading: null,
      alias: null,
    });
    expect(parseWikilinkInner("示例概念|密度")).toEqual({
      target: "示例概念",
      heading: null,
      alias: "密度",
    });
    expect(parseWikilinkInner("示例概念#要点")).toEqual({
      target: "示例概念",
      heading: "要点",
      alias: null,
    });
    expect(parseWikilinkInner(" 示例概念#要点|看要点 ")).toEqual({
      target: "示例概念",
      heading: "要点",
      alias: "看要点",
    });
  });

  it("accepts the table-escaped pipe and strips a .md suffix", () => {
    expect(parseWikilinkInner("示例概念\\|密度")).toEqual({
      target: "示例概念",
      heading: null,
      alias: "密度",
    });
    expect(parseWikilinkInner("wiki/概念/示例概念.md")).toEqual({
      target: "wiki/概念/示例概念",
      heading: null,
      alias: null,
    });
  });
});

describe("maskInlineCode", () => {
  it("blanks code spans but keeps the length", () => {
    const text = "a `[[x]]` b ``c ` [[y]]`` d [[z]]";
    const masked = maskInlineCode(text);
    expect(masked.length).toBe(text.length);
    expect(masked).not.toContain("[[x]]");
    expect(masked).not.toContain("[[y]]");
    expect(masked).toContain("[[z]]");
  });

  it("leaves unmatched backticks alone", () => {
    expect(maskInlineCode("单个 ` 反引号 [[链接]]")).toBe(
      "单个 ` 反引号 [[链接]]",
    );
  });

  it("ignores escaped backticks", () => {
    expect(maskInlineCode("\\`[[链接]]`")).toContain("[[链接]]");
  });
});

describe("extractWikilinks", () => {
  it("finds links with their line numbers", () => {
    const md = "第一行 [[甲]]\n第二行 [[乙|别名]] 和 ![[丙#小节]]\n";
    const links = extractWikilinks(md);
    expect(links).toEqual([
      {
        raw: "[[甲]]",
        target: "甲",
        heading: null,
        alias: null,
        embed: false,
        line: 0,
      },
      {
        raw: "[[乙|别名]]",
        target: "乙",
        heading: null,
        alias: "别名",
        embed: false,
        line: 1,
      },
      {
        raw: "![[丙#小节]]",
        target: "丙",
        heading: "小节",
        alias: null,
        embed: true,
        line: 1,
      },
    ]);
  });

  it("ignores links inside inline code and fenced code", () => {
    const md = [
      "用 `[[文件名]]` 互相链接",
      "```",
      "[[围栏]]",
      "```",
      "~~~",
      "[[波浪围栏]]",
      "~~~",
      "真的 [[链接]]",
    ].join("\n");
    expect(extractWikilinks(md).map((l) => l.target)).toEqual(["链接"]);
  });

  it("does not let a code span leak across paragraphs", () => {
    const md = "一个 ` 孤立的反引号\n\n下一段 [[链接]] 和 `代码`";
    expect(extractWikilinks(md).map((l) => l.target)).toEqual(["链接"]);
  });

  it("ignores empty and malformed links", () => {
    expect(extractWikilinks("[[]] [[ ]] [[a\nb]] [单个]")).toEqual([]);
  });
});

describe("basenameOf", () => {
  it("drops directories and the .md extension", () => {
    expect(basenameOf("wiki/概念/示例概念.md")).toBe("示例概念");
    expect(basenameOf("index.md")).toBe("index");
    expect(basenameOf("raw/a.b.md")).toBe("a.b");
  });
});

describe("buildLinkIndex", () => {
  const index = buildLinkIndex([
    "self/底色.md",
    "wiki/概念/示例概念.md",
    "wiki/书/示例概念.md",
    "self/待裁决.md",
    "self/待裁决/Q-001-努力.md",
    "README.md",
  ]);

  it("resolves by basename", () => {
    expect(index.resolve("底色")).toBe("self/底色.md");
    expect(index.resolve("待裁决")).toBe("self/待裁决.md");
    expect(index.resolve("Q-001-努力")).toBe("self/待裁决/Q-001-努力.md");
  });

  it("resolves path-qualified targets and case-insensitive fallbacks", () => {
    expect(index.resolve("wiki/书/示例概念")).toBe("wiki/书/示例概念.md");
    expect(index.resolve("readme")).toBe("README.md");
  });

  it("returns null for unknown targets", () => {
    expect(index.resolve("不存在")).toBeNull();
    expect(index.resolve("")).toBeNull();
  });

  it("reports duplicate basenames and prefers the first path in sort order", () => {
    expect(index.duplicates).toEqual([
      { name: "示例概念", paths: ["wiki/书/示例概念.md", "wiki/概念/示例概念.md"] },
    ]);
    expect(index.resolve("示例概念")).toBe("wiki/书/示例概念.md");
  });
});

describe("analyzeLinks", () => {
  const docs = Object.entries(fixtureKb).map(([path, content]) => ({
    path,
    content,
  }));
  const index = buildLinkIndex(docs.map((d) => d.path));
  const result = analyzeLinks(docs, index);

  it("reports only real broken links (not code or fenced code)", () => {
    expect(result.broken.map((b) => [b.from, b.link.target])).toEqual([
      ["self/底色.md", "没有这一页"],
    ]);
  });

  it("computes backlinks with one entry per linking document", () => {
    const toDise = result.backlinks.get("self/底色.md") ?? [];
    expect(toDise.map((b) => b.from)).toEqual([
      "index.md",
      "self/待裁决/Q-001-周会要不要取消.md",
      "self/待裁决/Q-003-反馈的频率.md",
      "systems/周节奏.md",
      "wiki/概念/信任的成本.md",
      "wiki/概念/写作即思考.md",
    ]);
    const toWriting = (result.backlinks.get("wiki/概念/写作即思考.md") ?? []).map(
      (b) => b.from,
    );
    expect(toWriting).toContain("wiki/概念/信任的成本.md");
    expect(toWriting).toContain("wiki/书/示例之书.md");
  });

  it("does not count self links as backlinks", () => {
    const self = analyzeLinks(
      [{ path: "a.md", content: "[[a]] [[#小节]]" }],
      buildLinkIndex(["a.md"]),
    );
    expect(self.backlinks.get("a.md") ?? []).toEqual([]);
    expect(self.broken).toEqual([]);
  });

  it("lists resolved outgoing links per document", () => {
    const out = result.outgoing.get("wiki/概念/信任的成本.md") ?? [];
    expect(out.map((o) => o.path)).toEqual([
      "wiki/概念/写作即思考.md",
      "self/底色.md",
    ]);
  });
});
