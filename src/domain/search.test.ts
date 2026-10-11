import { describe, expect, it } from "vitest";
import { buildKbModel } from "./kb";
import { makeSnippet, searchDocs, stripMarkdown } from "./search";
import { fixtureKb } from "./__fixtures__";

const model = buildKbModel(
  Object.entries(fixtureKb).map(([path, content]) => ({ path, content })),
);

describe("stripMarkdown", () => {
  it("removes markup but keeps the words and link labels", () => {
    expect(
      stripMarkdown("## 标题\n> **重要**：见 [[写作即思考#要点|写下来]] 和 [[小实验]]\n- [ ] 清单"),
    ).toBe("标题 重要：见 写下来 和 小实验 清单");
  });
});

describe("makeSnippet", () => {
  it("returns a window around the first match", () => {
    const text = "甲".repeat(50) + "关键字" + "乙".repeat(50);
    const snippet = makeSnippet(text, ["关键字"], 10);
    expect(snippet).toBe("…" + "甲".repeat(10) + "关键字" + "乙".repeat(10) + "…");
  });

  it("returns null when nothing matches", () => {
    expect(makeSnippet("abc", ["x"], 10)).toBeNull();
  });
});

describe("searchDocs", () => {
  it("returns nothing for a blank query", () => {
    expect(searchDocs(model.docs, "   ")).toEqual([]);
  });

  it("ranks title matches before body matches", () => {
    const hits = searchDocs(model.docs, "实验");
    expect(hits[0]?.doc.name).toBe("小实验");
    expect(hits[0]?.titleMatch).toBe(true);
    expect(hits.some((h) => !h.titleMatch)).toBe(true);
  });

  it("requires every term to match and ignores case", () => {
    const hits = searchDocs(model.docs, "信任 成本");
    expect(hits.map((h) => h.doc.name)).toContain("信任的成本");
    expect(hits.every((h) => /信任/.test(h.doc.content) && /成本/.test(h.doc.content))).toBe(
      true,
    );
    expect(searchDocs(model.docs, "AI 整理").length).toBeGreaterThan(0);
    expect(searchDocs(model.docs, "ai 整理").length).toBe(
      searchDocs(model.docs, "AI 整理").length,
    );
  });

  it("treats the page type and tags from frontmatter like the title", () => {
    const hits = searchDocs(model.docs, "判例");
    const top = hits.slice(0, 3).map((h) => h.doc.path);
    expect(top).toEqual([
      "self/判例/P-001-速度与退路.md",
      "self/判例/P-002-完整与及时.md",
      "self/判例/P-003-规则与例外.md",
    ]);
    expect(hits[0]?.titleMatch).toBe(true);
    expect(searchDocs(model.docs, "写作")[0]?.doc.name).toBe("写作即思考");
  });

  it("produces a snippet for full-text matches", () => {
    const hit = searchDocs(model.docs, "完全不存在的词语");
    expect(hit).toEqual([]);
    const body = searchDocs(model.docs, "重复确认");
    expect(body[0]?.doc.name).toBe("信任的成本");
    expect(body[0]?.snippet).toContain("重复确认");
  });
});
