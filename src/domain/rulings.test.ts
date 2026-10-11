import { describe, expect, it } from "vitest";
import { compareIds, isPendingStatus, parseRuling, sortRulings } from "./rulings";
import { fixture, fixtureKb } from "./__fixtures__";

describe("parseRuling", () => {
  it("reads all YAML fields, the title and the question", () => {
    const path = "self/待裁决/Q-001-周会要不要取消.md";
    const q = parseRuling(path, fixture(path));
    expect(q).toMatchObject({
      path,
      name: "Q-001-周会要不要取消",
      id: "Q-001",
      title: "周会要不要取消",
      status: "待裁决",
      pending: true,
      category: "新冲突",
      priority: "高",
      source: "2026-09-01 首次入库 I-02",
      updated: "2026-09-01",
      question: "周会保留还是取消？用你的话怎么说？",
    });
    expect(q.sections.map((s) => s.heading)).toEqual([
      "新材料",
      "张力",
      "AI 建议（参考）",
      "要你定",
    ]);
  });

  it("marks answered rulings as not pending", () => {
    const path = "self/待裁决/Q-005-休息日的消息.md";
    expect(parseRuling(path, fixture(path)).pending).toBe(false);
  });

  it("tolerates missing fields", () => {
    const q = parseRuling("self/待裁决/Q-077-草稿.md", "# Q-077 草稿\n");
    expect(q).toMatchObject({
      id: "Q-077",
      title: "草稿",
      status: null,
      pending: true,
      priority: null,
      question: null,
    });
  });
});

describe("isPendingStatus", () => {
  it("treats only resolved statuses as closed", () => {
    expect(isPendingStatus("待裁决")).toBe(true);
    expect(isPendingStatus(null)).toBe(true);
    expect(isPendingStatus("已裁决")).toBe(false);
    expect(isPendingStatus("已裁决（2026-09-15）")).toBe(false);
  });
});

describe("compareIds", () => {
  it("compares the numeric part naturally", () => {
    expect(["Q-010", "Q-002", "Q-001"].sort(compareIds)).toEqual([
      "Q-001",
      "Q-002",
      "Q-010",
    ]);
  });
});

describe("sortRulings", () => {
  it("sorts by priority (高, 中, 低, other) then by id", () => {
    const rulings = Object.entries(fixtureKb)
      .filter(([path]) => path.startsWith("self/待裁决/"))
      .map(([path, content]) => parseRuling(path, content));
    rulings.push(parseRuling("self/待裁决/Q-020-无优先级.md", "# Q-020 无优先级\n"));
    expect(sortRulings(rulings).map((q) => q.id)).toEqual([
      "Q-001",
      "Q-005",
      "Q-010",
      "Q-002",
      "Q-003",
      "Q-004",
      "Q-020",
    ]);
  });
});
