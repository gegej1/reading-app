import { describe, expect, it } from "vitest";
import { countRawItems, oneLiner, stripLeadingH1 } from "./pages";

describe("oneLiner", () => {
  it("reads the **一句话** line as plain text", () => {
    const body = "\n# 甲\n\n**一句话**：写下来就是 [[写作即思考|思考]] 的过程。\n\n## 要点\n";
    expect(oneLiner(body)).toBe("写下来就是 思考 的过程。");
  });

  it("returns null when the page has none", () => {
    expect(oneLiner("# 甲\n\n正文")).toBeNull();
  });
});

describe("countRawItems", () => {
  it("counts I-xx item headings outside code", () => {
    expect(countRawItems("## I-01\na\n## I-02\nb\n```\n## I-03\n```\n### I-04\n")).toBe(2);
    expect(countRawItems("没有条目")).toBe(0);
  });
});

describe("stripLeadingH1", () => {
  it("removes the first H1 when it opens the body", () => {
    expect(stripLeadingH1("\n# 标题\n\n正文\n")).toBe("\n\n正文\n");
    expect(stripLeadingH1("# 标题\r\n正文")).toBe("正文");
  });

  it("keeps bodies that start with something else", () => {
    expect(stripLeadingH1("正文\n# 后面的标题\n")).toBe("正文\n# 后面的标题\n");
    expect(stripLeadingH1("## 二级\n")).toBe("## 二级\n");
  });
});
