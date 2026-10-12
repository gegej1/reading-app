import { describe, expect, it } from "vitest";
import { diseEntryTexts, parseDise, pickDailyEntry, setConfirmed } from "./dise";
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

describe("setConfirmed", () => {
  const confirmedIn = (text: string, id: string) =>
    parseDise(text).entries.find((e) => e.id === id)?.confirmed;
  /** The lines that differ, as [before, after] pairs (the line count must not change). */
  const changedLines = (before: string, after: string) => {
    const a = before.split("\n");
    const b = after.split("\n");
    expect(b.length).toBe(a.length);
    return a.flatMap((line, i) => (line === b[i] ? [] : [[line, b[i]]]));
  };

  it("adds a trailing check mark to an unconfirmed entry only", () => {
    const next = setConfirmed(md, "D02", true);
    expect(next).toContain("### D02 同一时间只做一件主要的事 ✅\n");
    const before = md.split("\n");
    const after = next.split("\n");
    expect(after.length).toBe(before.length);
    const changed = after.filter((line, i) => line !== before[i]);
    expect(changed).toEqual(["### D02 同一时间只做一件主要的事 ✅"]);
    expect(confirmedIn(next, "D02")).toBe(true);
  });

  it("removes the check mark from a confirmed entry", () => {
    const next = setConfirmed(md, "D05", false);
    expect(next).toContain("### D05 先信任，再校准\n");
    expect(confirmedIn(next, "D05")).toBe(false);
  });

  it("returns the input unchanged when the entry is already in the desired state", () => {
    // A stale screen asks to confirm an entry that is already confirmed (and
    // the other way round): nothing may flip.
    expect(setConfirmed(md, "D05", true)).toBe(md);
    expect(setConfirmed(md, "D01", true)).toBe(md);
    expect(setConfirmed(md, "D02", false)).toBe(md);
    expect(setConfirmed(md, "D04", false)).toBe(md);
  });

  it("round-trips every entry that is not a draft to the identical string", () => {
    const entries = parseDise(md).entries.filter((entry) => !entry.draft);
    expect(entries.map((entry) => entry.id)).toEqual(["D01", "D02", "D03", "D05", "D06"]);
    for (const entry of entries) {
      const once = setConfirmed(md, entry.id, !entry.confirmed);
      expect(once, entry.id).not.toBe(md);
      expect(changedLines(md, once), entry.id).toHaveLength(1);
      expect(setConfirmed(once, entry.id, entry.confirmed), entry.id).toBe(md);
    }
  });

  it("preserves CRLF line endings and trailing spaces", () => {
    const crlf = "## 一、甲\r\n\r\n### D01 标题  \r\n> 引用\r\n";
    const once = setConfirmed(crlf, "D01", true);
    expect(once).toBe("## 一、甲\r\n\r\n### D01 标题 ✅  \r\n> 引用\r\n");
    expect(setConfirmed(once, "D01", true)).toBe(once);
    expect(setConfirmed(once, "D01", false)).toBe(crlf);
  });

  it("confirming a draft removes its marker and adds ✅ in one single-line edit", () => {
    const next = setConfirmed(md, "D04", true);
    expect(changedLines(md, next)).toEqual([["### D04 小步交付（草稿，待你确认）", "### D04 小步交付 ✅"]]);
    const d04 = parseDise(next).entries[3];
    expect(d04).toMatchObject({ id: "D04", title: "小步交付", confirmed: true, draft: false, notes: [] });
  });

  it("un-confirming a confirmed draft never puts the marker back, and the two states round-trip", () => {
    const confirmed = setConfirmed(md, "D04", true);
    const unconfirmed = setConfirmed(confirmed, "D04", false);
    expect(changedLines(confirmed, unconfirmed)).toEqual([["### D04 小步交付 ✅", "### D04 小步交付"]]);
    expect(unconfirmed).not.toContain("（草稿，待你确认）");
    expect(parseDise(unconfirmed).entries[3]).toMatchObject({ confirmed: false, draft: false });
    // From here on confirm and un-confirm restore each other exactly.
    expect(setConfirmed(unconfirmed, "D04", true)).toBe(confirmed);
    expect(setConfirmed(setConfirmed(unconfirmed, "D04", true), "D04", false)).toBe(unconfirmed);
  });

  it("is idempotent for drafts too", () => {
    const once = setConfirmed(md, "D04", true);
    expect(setConfirmed(once, "D04", true)).toBe(once);
    // Un-confirming an unconfirmed draft changes nothing: the marker stays.
    expect(setConfirmed(md, "D04", false)).toBe(md);
    // A heading that has both (written by hand) is already confirmed.
    const both = "### D04 小步交付（草稿，待你确认） ✅\n";
    expect(setConfirmed(both, "D04", true)).toBe(both);
    expect(setConfirmed(both, "D04", false)).toBe("### D04 小步交付（草稿，待你确认）\n");
  });

  it("recognises the marker with ASCII punctuation, spaces and other notes, and keeps CRLF", () => {
    const cases: [string, string][] = [
      ["### D07 慢一点 (草稿, 待你确认)\n", "### D07 慢一点 ✅\n"],
      ["### D07 慢一点 （ 草稿，待你确认 ）\n", "### D07 慢一点 ✅\n"],
      ["### D07 慢一点（草稿，待你确认）（认同）\n", "### D07 慢一点（认同） ✅\n"],
      ["### D07 慢一点（认同）（草稿，待你确认）\n", "### D07 慢一点（认同） ✅\n"],
      ["### D07 慢一点（草稿，待你确认）  \r\n> 原话\r\n", "### D07 慢一点 ✅  \r\n> 原话\r\n"],
    ];
    for (const [before, after] of cases) {
      expect(parseDise(before).entries[0]?.draft, before).toBe(true);
      expect(setConfirmed(before, "D07", true), before).toBe(after);
    }
    expect(parseDise("### D07 慢一点（认同）（草稿，待你确认）\n").entries[0]).toMatchObject({
      title: "慢一点",
      notes: ["认同"],
      draft: true,
    });
    // Other notes about drafts are notes, not the marker.
    expect(parseDise("### D08 甲（草稿）\n").entries[0]).toMatchObject({ draft: false, notes: ["草稿"] });
  });

  it("tolerates the emoji variation selector when un-confirming", () => {
    const text = "### D01 标题 ✅\uFE0F\n";
    expect(setConfirmed(text, "D01", true)).toBe(text);
    expect(setConfirmed(text, "D01", false)).toBe("### D01 标题\n");
  });

  it("does not match a longer id, a heading inside code or the frontmatter", () => {
    const text = "### D010 别的\n```\n### D01 代码\n```\n### D01 真的\n";
    expect(setConfirmed(text, "D01", true)).toBe(
      "### D010 别的\n```\n### D01 代码\n```\n### D01 真的 ✅\n",
    );
    const withFrontmatter = "---\n说明: |\n  ### D01 不是标题\n---\n### D01 真的\n";
    expect(setConfirmed(withFrontmatter, "D01", true)).toBe(
      "---\n说明: |\n  ### D01 不是标题\n---\n### D01 真的 ✅\n",
    );
  });

  it("throws when the entry does not exist, whatever the desired state", () => {
    expect(() => setConfirmed(md, "D42", true)).toThrow(/D42/);
    expect(() => setConfirmed(md, "D42", false)).toThrow(/D42/);
  });
});

describe("diseEntryTexts", () => {
  it("returns every entry's exact source text, heading included", () => {
    const texts = diseEntryTexts(md);
    expect(texts.map((t) => [t.id, t.confirmed])).toEqual([
      ["D01", true],
      ["D02", false],
      ["D03", false],
      ["D04", false],
      ["D05", true],
      ["D06", false],
    ]);
    expect(texts[0]?.text).toBe(
      [
        "### D01 先留退路，再加速度 ✅",
        "> 先确认能退回来，再决定跑多快。退路想清楚了，速度才敢放开。",
        "",
        "- 可以撤回的决定，先用小实验试一试。",
        "- 撤不回来的决定，多等一周也值得。",
        "- 判例：[[P-001-速度与退路]]",
      ].join("\n"),
    );
  });

  it("ends an entry at the next section heading and keeps code blocks inside it", () => {
    const texts = diseEntryTexts(md);
    // D02 is the last entry of section 一: it stops before `## 二、…`.
    expect(texts[1]?.text.endsWith("- 来源：示例笔记 S1.2")).toBe(true);
    // The fenced `### D99` belongs to D06, it is not an entry of its own.
    expect(texts[5]?.text).toContain("### D99 代码块里的标题不是条目");
  });

  it("ignores blank lines between entries, so appending an entry changes no other text", () => {
    const before = "## 一、甲\n\n### D01 甲 ✅\n> 原话\n";
    const after = `${before}\n### D02 新的（草稿，待你确认）\n> 新原话\n`;
    expect(diseEntryTexts(after)[0]).toEqual(diseEntryTexts(before)[0]);
    expect(diseEntryTexts(before)[0]?.text).toBe("### D01 甲 ✅\n> 原话");
  });

  it("keeps the text byte-exact: CR and trailing spaces stay", () => {
    const text = "### D01 甲 ✅  \r\n> 原话\r\n\r\n";
    expect(diseEntryTexts(text)[0]?.text).toBe("### D01 甲 ✅  \r\n> 原话\r");
  });

  it("skips the frontmatter and returns nothing for a file without entries", () => {
    expect(diseEntryTexts("---\n类型: 底色\n---\n# 底色\n")).toEqual([]);
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
