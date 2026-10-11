import { describe, expect, it } from "vitest";
import { buildKbModel } from "../../domain";
import { fixtureKb } from "../../domain/__fixtures__";
import { captureSummary, docDate, groupRaw } from "./rawGroups";

const model = buildKbModel(Object.entries(fixtureKb).map(([path, content]) => ({ path, content })));

describe("docDate", () => {
  it("prefers the date in the file name, then frontmatter dates", () => {
    const inbox = model.byPath.get("raw/inbox/2026-09-01-收件箱.md");
    expect(inbox && docDate(inbox)).toBe("2026-09-01");
    const book = {
      name: "某本书",
      frontmatter: { 抓取: "2026-10-12（整理稿）" },
    };
    expect(docDate(book)).toBe("2026-10-12");
    expect(docDate({ name: "没有日期", frontmatter: {} })).toBeNull();
  });
});

describe("groupRaw", () => {
  it("orders 收件箱, 对话, 书 first and sorts each group newest first", () => {
    const docs = [
      ...model.raw,
      { ...model.raw[0]!, path: "raw/books/甲.md", name: "甲", subgroup: "books", frontmatter: {} },
      { ...model.raw[0]!, path: "raw/inbox/2026-09-09-收件箱.md", name: "2026-09-09-收件箱" },
    ];
    const groups = groupRaw(docs);
    expect(groups.map((g) => g.label)).toEqual(["收件箱", "对话", "书"]);
    expect(groups[0]?.docs.map((d) => d.name)).toEqual(["2026-09-09-收件箱", "2026-09-01-收件箱"]);
  });
});

describe("captureSummary", () => {
  it("summarises a capture by its first words", () => {
    const capture = model.byPath.get("inbox/2026-09-10-想法.md");
    expect(capture && captureSummary(capture)).toBe("晚上散步的时候想到：清单越短，越容易做完。");
  });
});
