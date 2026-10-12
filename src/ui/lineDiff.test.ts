import { describe, expect, it } from "vitest";
import { diffBlocks, diffFiles, splitMark, type DiffLine } from "./lineDiff";

const shape = (lines: DiffLine[]) => lines.map((line) => `${line.kind === "same" ? " " : line.kind === "add" ? "+" : "-"}${line.text}`);

/** Applies a diff to its old side: must give the new side (checks the edit script). */
function rebuild(lines: DiffLine[]) {
  return {
    before: lines.filter((line) => line.kind !== "add").map((line) => line.text),
    after: lines.filter((line) => line.kind !== "del").map((line) => line.text),
  };
}

describe("diffFiles", () => {
  it("finds a changed line among unchanged ones, with line numbers on both sides", () => {
    const diff = diffFiles("甲\n乙\n丙\n丁\n", "甲\n乙二\n丙\n丁\n戊\n");
    expect(shape(diff.lines)).toEqual([" 甲", "-乙", "+乙二", " 丙", " 丁", "+戊"]);
    expect(diff.lines.map((line) => [line.oldNo, line.newNo])).toEqual([
      [1, 1],
      [2, null],
      [null, 2],
      [3, 3],
      [4, 4],
      [null, 5],
    ]);
    expect([diff.added, diff.removed]).toEqual([2, 1]);
    expect(diff.endNewlineChanged).toBe(false);
  });

  it("marks the one character that changed inside a line, by code point", () => {
    const diff = diffFiles("> 工具是给人用的，不是给表格用的。\n", "> 工具是给人用的，不是给报表用的。\n");
    const [removed, added] = diff.lines;
    expect(splitMark(removed ?? { text: "", mark: null })).toEqual({ before: "> 工具是给人用的，不是给", marked: "表格", after: "用的。" });
    expect(splitMark(added ?? { text: "", mark: null })).toEqual({ before: "> 工具是给人用的，不是给", marked: "报表", after: "用的。" });
    // Characters outside the BMP count as one.
    const emoji = diffFiles("a😀b\n", "a😃b\n").lines[1];
    expect(emoji?.mark).toEqual([1, 2]);
    // Lines with little in common are not marked.
    expect(diffFiles("完全不同的一行\n", "另一句话\n").lines.every((line) => line.mark === null)).toBe(true);
  });

  it("shows new and deleted files as all added or all removed", () => {
    expect(shape(diffFiles(null, "新\n文件\n").lines)).toEqual(["+新", "+文件"]);
    expect(shape(diffFiles("旧\n", null).lines)).toEqual(["-旧"]);
    expect(diffFiles("", "").lines).toEqual([]);
  });

  it("notices a changed final newline and keeps CR inside lines", () => {
    expect(diffFiles("甲\n", "甲").endNewlineChanged).toBe(true);
    const crlf = diffFiles("甲\r\n乙\r\n", "甲\n乙\r\n");
    expect(shape(crlf.lines)).toEqual(["-甲\r", "+甲", " 乙\r"]);
  });

  it("produces a valid edit script for random edits", () => {
    let seed = 7;
    const random = () => {
      seed = (seed * 1103515245 + 12345) % 2147483648;
      return seed / 2147483648;
    };
    for (let round = 0; round < 60; round += 1) {
      const a = Array.from({ length: Math.floor(random() * 30) }, () => String.fromCharCode(0x4e00 + Math.floor(random() * 6)));
      const b = a.filter(() => random() > 0.2).map((line) => (random() > 0.85 ? `${line}改` : line));
      if (random() > 0.5) b.splice(Math.floor(random() * (b.length + 1)), 0, "新增");
      const diff = diffFiles(a.join("\n"), b.join("\n"));
      expect(rebuild(diff.lines)).toEqual({ before: a, after: b });
    }
  });

  it("gives up on hopelessly different large files and shows them as replaced", () => {
    const a = Array.from({ length: 2500 }, (_, i) => `旧 ${i}`).join("\n");
    const b = Array.from({ length: 2500 }, (_, i) => `新 ${i}`).join("\n");
    const diff = diffFiles(a, b);
    expect(diff.tooLarge).toBe(true);
    expect([diff.removed, diff.added]).toEqual([2500, 2500]);
  });
});

describe("diffBlocks", () => {
  it("keeps three lines of context and folds longer unchanged runs into gaps", () => {
    const before = Array.from({ length: 20 }, (_, i) => `行${i + 1}`);
    const after = [...before];
    after[9] = "行10改";
    const blocks = diffBlocks(diffFiles(before.join("\n"), after.join("\n")).lines);
    expect(blocks.map((block) => [block.kind, block.lines.length])).toEqual([
      ["gap", 6],
      ["lines", 8],
      ["gap", 7],
    ]);
    expect(shape(blocks[1]?.lines ?? [])).toEqual([" 行7", " 行8", " 行9", "-行10", "+行10改", " 行11", " 行12", " 行13"]);
  });

  it("keeps everything when the file is short", () => {
    const blocks = diffBlocks(diffFiles("a\nb\n", "a\nc\n").lines);
    expect(blocks.map((block) => block.kind)).toEqual(["lines"]);
  });
});
