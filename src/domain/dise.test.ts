import { describe, expect, it } from "vitest";
import { parseDise, pickDailyEntry, toggleConfirmed } from "./dise";
import { fixture } from "./__fixtures__";

const md = fixture("self/底色.md");

describe("parseDise", () => {
  const doc = parseDise(md);

  it("reads frontmatter, title and the intro", () => {
    expect(doc.frontmatter["版本"]).toBe("v0.3");
    expect(doc.title).toBe("示例底色 v0.3");
    expect(doc.introMarkdown).toContain("这是演示用的虚构数据");
  });

  it("parses sections with their ordinal", () => {
    expect(
      doc.sections.map((s) => [s.ordinal, s.title, s.entries.length]),
    ).toEqual([
      ["一", "怎么做选择", 2],
      ["二", "怎么做事", 2],
      ["三", "怎么待人", 2],
    ]);
  });

  it("parses every entry in document order and skips headings in code", () => {
    expect(doc.entries.map((e) => e.id)).toEqual([
      "D01",
      "D02",
      "D03",
      "D04",
      "D05",
      "D06",
    ]);
  });

  it("detects confirmation, drafts and trailing notes", () => {
    const byId = Object.fromEntries(doc.entries.map((e) => [e.id, e]));
    expect(byId.D01).toMatchObject({
      title: "先留退路，再加速度",
      confirmed: true,
      draft: false,
      notes: [],
    });
    expect(byId.D03).toMatchObject({
      title: "写下来才算想清楚",
      rawTitle: "写下来才算想清楚（认同）",
      confirmed: false,
      draft: false,
      notes: ["认同"],
    });
    expect(byId.D04).toMatchObject({
      title: "小步交付",
      confirmed: false,
      draft: true,
      notes: [],
    });
    expect(byId.D06?.notes).toEqual(["待你填写"]);
  });

  it("separates the quote from the rest of the entry", () => {
    const d01 = doc.entries[0];
    expect(d01?.quote).toBe(
      "先确认能退回来，再决定跑多快。退路想清楚了，速度才敢放开。",
    );
    expect(d01?.bodyMarkdown).toBe(
      [
        "- 可以撤回的决定，先用小实验试一试。",
        "- 撤不回来的决定，多等一周也值得。",
        "- 判例：[[P-001-速度与退路]]",
      ].join("\n"),
    );
    expect(d01?.sectionIndex).toBe(0);
    expect(d01?.line).toBeGreaterThan(0);
  });

  it("keeps fenced code out of the last entry's quote but in its body", () => {
    const d06 = doc.entries[5];
    expect(d06?.quote).toBe("候选：说清楚哪件事、哪个结果、下次怎么做。");
    expect(d06?.bodyMarkdown).toContain("[[Q-003-反馈的频率]]");
    expect(d06?.bodyMarkdown).toContain("### D99 代码块里的标题不是条目");
  });

  it("joins multi-line quotes and tolerates entries without a quote", () => {
    const doc2 = parseDise(
      "## 一、甲\n\n### D01 第一条\n> 第一行\n> 第二行\n>\n> 第三段\n\n- 要点\n\n### D02 没有引用\n- 只有要点\n",
    );
    expect(doc2.entries[0]?.quote).toBe("第一行\n第二行\n\n第三段");
    expect(doc2.entries[0]?.bodyMarkdown).toBe("- 要点");
    expect(doc2.entries[1]?.quote).toBe("");
    expect(doc2.entries[1]?.bodyMarkdown).toBe("- 只有要点");
  });

  it("handles a document without sections", () => {
    const doc3 = parseDise("# 空的底色\n");
    expect(doc3.sections).toEqual([]);
    expect(doc3.entries).toEqual([]);
    expect(doc3.title).toBe("空的底色");
  });
});

describe("toggleConfirmed", () => {
  it("adds a trailing check mark to an unconfirmed entry only", () => {
    const next = toggleConfirmed(md, "D02");
    expect(next).toContain("### D02 同一时间只做一件主要的事 ✅\n");
    const before = md.split("\n");
    const after = next.split("\n");
    expect(after.length).toBe(before.length);
    const changed = after.filter((line, i) => line !== before[i]);
    expect(changed).toEqual(["### D02 同一时间只做一件主要的事 ✅"]);
    expect(parseDise(next).entries[1]?.confirmed).toBe(true);
  });

  it("removes the check mark from a confirmed entry", () => {
    const next = toggleConfirmed(md, "D05");
    expect(next).toContain("### D05 先信任，再校准\n");
    expect(parseDise(next).entries[4]?.confirmed).toBe(false);
  });

  it("returns the identical string after toggling twice", () => {
    for (const id of ["D01", "D02", "D03", "D04", "D05", "D06"]) {
      expect(toggleConfirmed(toggleConfirmed(md, id), id)).toBe(md);
    }
  });

  it("preserves CRLF line endings and trailing spaces", () => {
    const crlf = "## 一、甲\r\n\r\n### D01 标题  \r\n> 引用\r\n";
    const once = toggleConfirmed(crlf, "D01");
    expect(once).toBe("## 一、甲\r\n\r\n### D01 标题 ✅  \r\n> 引用\r\n");
    expect(toggleConfirmed(once, "D01")).toBe(crlf);
  });

  it("keeps the draft marker when confirming a draft", () => {
    const next = toggleConfirmed(md, "D04");
    expect(next).toContain("### D04 小步交付（草稿，待你确认） ✅\n");
    const d04 = parseDise(next).entries[3];
    expect(d04?.confirmed).toBe(true);
    expect(d04?.draft).toBe(true);
  });

  it("does not match a longer id or a heading inside code", () => {
    const text = "### D010 别的\n```\n### D01 代码\n```\n### D01 真的\n";
    expect(toggleConfirmed(text, "D01")).toBe(
      "### D010 别的\n```\n### D01 代码\n```\n### D01 真的 ✅\n",
    );
  });

  it("throws when the entry does not exist", () => {
    expect(() => toggleConfirmed(md, "D42")).toThrow(/D42/);
  });
});

describe("pickDailyEntry", () => {
  const entries = ["a", "b", "c"];

  it("is deterministic for a calendar day and rotates day by day", () => {
    const morning = new Date(2026, 9, 12, 8, 0);
    const evening = new Date(2026, 9, 12, 23, 59);
    const tomorrow = new Date(2026, 9, 13, 8, 0);
    expect(pickDailyEntry(entries, morning)).toBe(
      pickDailyEntry(entries, evening),
    );
    const i = entries.indexOf(pickDailyEntry(entries, morning) ?? "");
    expect(pickDailyEntry(entries, tomorrow)).toBe(
      entries[(i + 1) % entries.length],
    );
  });

  it("returns null for an empty list", () => {
    expect(pickDailyEntry([], new Date())).toBeNull();
  });
});
