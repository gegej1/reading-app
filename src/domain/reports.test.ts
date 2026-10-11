import { describe, expect, it } from "vitest";
import {
  parseAuditLine,
  parseReport,
  reportDateFromPath,
  setChecked,
  sortReportsNewestFirst,
} from "./reports";
import { fixture, fixtureKb } from "./__fixtures__";

const firstPath = "reports/2026-09-01-首次入库报告.md";
const secondPath = "reports/2026-09-08-入库报告-2.md";

describe("parseAuditLine", () => {
  it("splits summary, tension, ruling and precedent", () => {
    expect(
      parseAuditLine(
        'I-12 摘要文字 ↔ 张力"引号"：AI 的裁决，理由（依据 [[P-009-示例判例]]）',
      ),
    ).toEqual({
      itemId: "I-12",
      summary: "摘要文字",
      tension: '张力"引号"',
      ruling: "AI 的裁决，理由",
      precedent: "P-009-示例判例",
    });
  });

  it("falls back gracefully when parts are missing", () => {
    expect(parseAuditLine("I-08 先写清单再出门采购")).toEqual({
      itemId: "I-08",
      summary: "先写清单再出门采购",
      tension: null,
      ruling: null,
      precedent: null,
    });
    expect(parseAuditLine("没有编号的一条")).toEqual({
      itemId: null,
      summary: "没有编号的一条",
      tension: null,
      ruling: null,
      precedent: null,
    });
  });

  it("accepts ASCII parentheses and an aliased precedent link", () => {
    expect(
      parseAuditLine("I-3 甲 ↔ 乙：丙 (依据 [[P-001-速度与退路|P-001]])")
        .precedent,
    ).toBe("P-001-速度与退路");
  });
});

describe("parseReport", () => {
  it("reads the date, title, result line and audit items", () => {
    const report = parseReport(firstPath, fixture(firstPath));
    expect(report).toMatchObject({
      path: firstPath,
      name: "2026-09-01-首次入库报告",
      date: "2026-09-01",
      sequence: 1,
      title: "首次入库报告（2026-09-01）",
      resultLine: "自动入库 2 条｜按判例裁决 2 条（待你抽查）｜交给你裁决 1 条",
      resultParts: [
        "自动入库 2 条",
        "按判例裁决 2 条（待你抽查）",
        "交给你裁决 1 条",
      ],
    });
    expect(report.auditItems).toHaveLength(2);
    expect(report.auditItems[0]).toMatchObject({
      itemId: "I-01",
      checked: true,
      summary: "小实验比大计划更快找到答案",
      tension: 'D02"只做一件主要的事"',
      ruling: "不冲突，实验服务于主线",
      precedent: "P-001-速度与退路",
      reportPath: firstPath,
    });
    expect(report.auditItems[1]).toMatchObject({
      itemId: "I-03",
      checked: false,
      precedent: "P-002-完整与及时",
    });
    expect(report.auditItems[1]?.text).toBe(
      'I-03 先交草图再交成品 ↔ "一次做完返工少"：先交草图，因为有人在等（依据 [[P-002-完整与及时]]）',
    );
  });

  it("only collects checklist items from the 待你抽查 section", () => {
    const report = parseReport(secondPath, fixture(secondPath));
    expect(report.sequence).toBe(2);
    expect(report.auditItems.map((i) => [i.itemId, i.checked])).toEqual([
      ["I-07", false],
      ["I-08", true],
      [null, false],
    ]);
  });

  it("handles reports without a result line or audit section", () => {
    const report = parseReport("reports/笔记.md", "# 随便写写\n");
    expect(report).toMatchObject({
      date: null,
      sequence: 1,
      title: "随便写写",
      resultLine: null,
      resultParts: [],
      auditItems: [],
    });
  });
});

describe("reportDateFromPath", () => {
  it("extracts the date and the same-day sequence number", () => {
    expect(reportDateFromPath("reports/2026-09-01-首次入库报告.md")).toEqual({
      date: "2026-09-01",
      sequence: 1,
    });
    expect(reportDateFromPath("reports/2026-09-01-入库报告-2.md")).toEqual({
      date: "2026-09-01",
      sequence: 2,
    });
    expect(reportDateFromPath("reports/总结.md")).toEqual({
      date: null,
      sequence: 1,
    });
  });
});

describe("sortReportsNewestFirst", () => {
  it("orders by date, then by same-day sequence, newest first", () => {
    const reports = Object.entries(fixtureKb)
      .filter(([path]) => path.startsWith("reports/"))
      .map(([path, content]) => parseReport(path, content));
    expect(sortReportsNewestFirst(reports).map((r) => r.name)).toEqual([
      "2026-09-08-入库报告-2",
      "2026-09-08-入库报告",
      "2026-09-01-首次入库报告",
    ]);
  });
});

describe("setChecked", () => {
  const md = fixture(firstPath);
  const checkedIn = (text: string, itemId: string) =>
    parseReport(firstPath, text).auditItems.find((i) => i.itemId === itemId)?.checked;

  it("checks an unchecked item and changes only that line", () => {
    const next = setChecked(md, "I-03", true);
    const before = md.split("\n");
    const after = next.split("\n");
    const changed = after.filter((line, i) => line !== before[i]);
    expect(changed).toHaveLength(1);
    expect(changed[0]?.startsWith("- [x] I-03 ")).toBe(true);
    expect(checkedIn(next, "I-03")).toBe(true);
  });

  it("unchecks a checked item", () => {
    const next = setChecked(md, "I-01", false);
    expect(next).toContain("- [ ] I-01 小实验比大计划更快找到答案");
    expect(checkedIn(next, "I-01")).toBe(false);
  });

  it("returns the input unchanged when the item is already in the desired state", () => {
    expect(setChecked(md, "I-01", true)).toBe(md);
    expect(setChecked(md, "I-03", false)).toBe(md);
    expect(setChecked("## 待你抽查\n- [X] I-02 乙\n", "I-02", true)).toBe(
      "## 待你抽查\n- [X] I-02 乙\n",
    );
  });

  it("round-trips every item to the identical string", () => {
    for (const item of parseReport(firstPath, md).auditItems) {
      if (!item.itemId) continue;
      const once = setChecked(md, item.itemId, !item.checked);
      expect(once, item.itemId).not.toBe(md);
      expect(setChecked(once, item.itemId, item.checked), item.itemId).toBe(md);
    }
  });

  it("ignores lines outside the audit section and requires an exact id", () => {
    const second = fixture(secondPath);
    expect(() => setChecked(second, "I-99", true)).toThrow(/I-99/);
    expect(() => setChecked(md, "I-0", true)).toThrow();
  });

  it("keeps CRLF line endings and ignores frontmatter", () => {
    const text = "---\r\n类型: 入库报告\r\n---\r\n## 待你抽查\r\n- [ ] I-02 乙\r\n";
    const once = setChecked(text, "I-02", true);
    expect(once).toBe("---\r\n类型: 入库报告\r\n---\r\n## 待你抽查\r\n- [x] I-02 乙\r\n");
    expect(setChecked(once, "I-02", false)).toBe(text);
    expect(parseReport("reports/x.md", once).auditItems[0]).toMatchObject({
      itemId: "I-02",
      checked: true,
      line: 4,
    });
  });

  it("does not mistake I-1 for I-12", () => {
    const text = "## 待你抽查\n\n- [ ] I-12 甲\n- [ ] I-1 乙\n";
    expect(setChecked(text, "I-1", true)).toBe(
      "## 待你抽查\n\n- [ ] I-12 甲\n- [x] I-1 乙\n",
    );
  });
});
