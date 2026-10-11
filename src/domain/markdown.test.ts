import { describe, expect, it } from "vitest";
import {
  fencedCodeMask,
  firstHeading,
  parseHeading,
  splitLines,
  splitSections,
} from "./markdown";

describe("splitLines", () => {
  it("splits on LF so that joining with LF restores the text", () => {
    const text = "a\r\nb\n\nc";
    const lines = splitLines(text);
    expect(lines).toEqual(["a\r", "b", "", "c"]);
    expect(lines.join("\n")).toBe(text);
  });
});

describe("parseHeading", () => {
  it("recognises ATX headings and strips closing hashes", () => {
    expect(parseHeading("## 一、怎么做选择")).toEqual({
      level: 2,
      text: "一、怎么做选择",
    });
    expect(parseHeading("### D01 标题 ✅  ")).toEqual({
      level: 3,
      text: "D01 标题 ✅",
    });
    expect(parseHeading("# 标题 ##")).toEqual({ level: 1, text: "标题" });
    expect(parseHeading("#没有空格")).toBeNull();
    expect(parseHeading("    # 缩进代码")).toBeNull();
    expect(parseHeading("正文")).toBeNull();
  });
});

describe("fencedCodeMask", () => {
  it("marks fenced code lines including the fences", () => {
    const lines = splitLines(
      "正文\n```text\n### D99 不是条目\n```\n后文\n~~~~\n```\n~~~~\n结束",
    );
    expect(fencedCodeMask(lines)).toEqual([
      false,
      true,
      true,
      true,
      false,
      true,
      true,
      true,
      false,
    ]);
  });

  it("keeps an unclosed fence open until the end of the document", () => {
    const lines = splitLines("a\n```\nb\nc");
    expect(fencedCodeMask(lines)).toEqual([false, true, true, true]);
  });

  it("requires the closing fence to be at least as long as the opening one", () => {
    const lines = splitLines("````\n```\n````\nx");
    expect(fencedCodeMask(lines)).toEqual([true, true, true, false]);
  });
});

describe("splitSections", () => {
  const body = [
    "# 文档标题",
    "",
    "> 前言",
    "",
    "## 冲突",
    "- 一",
    "",
    "### 小节",
    "细节",
    "",
    "## 裁决",
    "```md",
    "## 代码里的标题",
    "```",
    "结论",
    "",
  ].join("\n");

  it("splits on headings of the given level and keeps deeper headings inside", () => {
    const { preamble, sections } = splitSections(body, 2);
    expect(preamble).toBe("# 文档标题\n\n> 前言");
    expect(sections.map((s) => s.heading)).toEqual(["冲突", "裁决"]);
    expect(sections[0]?.markdown).toBe("- 一\n\n### 小节\n细节");
    expect(sections[1]?.markdown).toBe("```md\n## 代码里的标题\n```\n结论");
    expect(sections[0]?.line).toBe(4);
    expect(sections[1]?.line).toBe(10);
  });

  it("ends level-3 sections at the next level-2 heading", () => {
    const md = "## 一\n### A\na\n## 二\n### B\nb";
    const { sections } = splitSections(md, 3);
    expect(sections.map((s) => [s.heading, s.markdown])).toEqual([
      ["A", "a"],
      ["B", "b"],
    ]);
  });
});

describe("firstHeading", () => {
  it("returns the first heading of a level outside code", () => {
    expect(firstHeading("```\n# 不是\n```\n# 是这个\n# 第二个", 1)).toBe(
      "是这个",
    );
    expect(firstHeading("正文", 1)).toBeNull();
  });
});
