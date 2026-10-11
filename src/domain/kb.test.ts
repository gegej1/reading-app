import { describe, expect, it } from "vitest";
import { buildKbModel, classifyPath, docTitle, loadKb } from "./kb";
import { fixtureKb } from "./__fixtures__";
import { createMemoryStore } from "../store/memoryStore";
import type { KbStore } from "../store/types";

const files = Object.entries(fixtureKb).map(([path, content]) => ({
  path,
  content,
}));

describe("classifyPath", () => {
  it("assigns every KB area to a group", () => {
    expect(classifyPath("self/底色.md")).toEqual({ group: "dise", subgroup: null });
    expect(classifyPath("self/判例/P-001-x.md")).toEqual({
      group: "precedent",
      subgroup: null,
    });
    expect(classifyPath("self/待裁决/Q-001-x.md")).toEqual({
      group: "ruling",
      subgroup: null,
    });
    expect(classifyPath("self/待裁决.md")).toEqual({
      group: "rulingIndex",
      subgroup: null,
    });
    expect(classifyPath("reports/2026-09-01-x.md")).toEqual({
      group: "report",
      subgroup: null,
    });
    expect(classifyPath("wiki/概念/x.md")).toEqual({ group: "wiki", subgroup: "概念" });
    expect(classifyPath("wiki/x.md")).toEqual({ group: "wiki", subgroup: null });
    expect(classifyPath("systems/x.md")).toEqual({ group: "system", subgroup: null });
    expect(classifyPath("raw/books/x.md")).toEqual({ group: "raw", subgroup: "books" });
    expect(classifyPath("inbox/x.md")).toEqual({ group: "capture", subgroup: null });
    expect(classifyPath("inbox/裁决/x.md")).toEqual({ group: "answer", subgroup: null });
    expect(classifyPath("inbox/别的/x.md")).toEqual({ group: "other", subgroup: null });
    expect(classifyPath("index.md")).toEqual({ group: "other", subgroup: null });
    expect(classifyPath("self/决策日志.md")).toEqual({ group: "self", subgroup: null });
  });
});

describe("docTitle", () => {
  it("prefers the first H1 and falls back to the file name", () => {
    expect(docTitle("wiki/概念/a.md", "\n# 标题一\n正文")).toBe("标题一");
    expect(docTitle("wiki/概念/a.md", "没有标题")).toBe("a");
  });
});

describe("buildKbModel", () => {
  const model = buildKbModel(files);

  it("indexes every document by path", () => {
    expect(model.docs).toHaveLength(files.length);
    expect(model.byPath.get("wiki/概念/小实验.md")?.title).toBe("小实验");
    expect(model.byName.get("小实验")?.path).toBe("wiki/概念/小实验.md");
  });

  it("parses 底色", () => {
    expect(model.disePath).toBe("self/底色.md");
    expect(model.dise?.entries.map((e) => e.id)).toEqual([
      "D01",
      "D02",
      "D03",
      "D04",
      "D05",
      "D06",
    ]);
  });

  it("parses precedents sorted by id", () => {
    expect(model.precedents.map((p) => p.id)).toEqual(["P-001", "P-002", "P-003"]);
  });

  it("parses rulings and keeps pending ones sorted by priority", () => {
    expect(model.rulings).toHaveLength(6);
    expect(model.pendingRulings.map((q) => q.id)).toEqual([
      "Q-001",
      "Q-010",
      "Q-002",
      "Q-003",
      "Q-004",
    ]);
  });

  it("sorts reports newest first and flattens audit items", () => {
    expect(model.reports.map((r) => r.name)).toEqual([
      "2026-09-08-入库报告-2",
      "2026-09-08-入库报告",
      "2026-09-01-首次入库报告",
    ]);
    expect(model.auditItems.map((i) => i.itemId)).toEqual([
      "I-07",
      "I-08",
      null,
      "I-05",
      "I-01",
      "I-03",
    ]);
  });

  it("groups wiki pages by folder in a stable order", () => {
    expect(
      model.wikiGroups.map((g) => [g.name, g.docs.map((d) => d.name)]),
    ).toEqual([
      ["概念", ["信任的成本", "写作即思考", "小实验"]],
      ["人物与公司", ["青柳工作室"]],
      ["书", ["示例之书"]],
    ]);
    expect(model.systems.map((d) => d.name)).toEqual(["周节奏"]);
  });

  it("collects raw files, captures and answered rulings", () => {
    expect(model.raw.map((d) => d.path)).toEqual([
      "raw/inbox/2026-09-01-收件箱.md",
      "raw/对话/2026-09-01-示例对话.md",
    ]);
    expect(model.captures.map((d) => d.name)).toEqual([
      "2026-09-10-想法",
      "2026-09-11-摘抄",
    ]);
    expect(model.answers.map((d) => d.name)).toEqual(["2026-09-12-Q-001"]);
  });

  it("computes links, backlinks and broken links", () => {
    expect(model.links.broken.map((b) => b.link.target)).toEqual(["没有这一页"]);
    expect(model.links.backlinks.get("self/判例/P-001-速度与退路.md")?.length).toBe(
      5,
    );
  });

  it("computes dashboard counts", () => {
    expect(model.dashboard).toEqual({
      inboxCaptures: 2,
      answeredRulings: 1,
      pendingRulings: { total: 5, high: 2, medium: 2, low: 1, other: 0 },
      uncheckedAudit: 4,
      latestReport: {
        path: "reports/2026-09-08-入库报告-2.md",
        name: "2026-09-08-入库报告-2",
        date: "2026-09-08",
        title: "入库报告（2026-09-08 第二次）",
        resultLine: "自动入库 2 条｜按判例裁决 2 条（待你抽查）｜交给你裁决 0 条",
        resultParts: ["自动入库 2 条", "按判例裁决 2 条（待你抽查）", "交给你裁决 0 条"],
      },
    });
  });

  it("works for an empty knowledge base", () => {
    const empty = buildKbModel([]);
    expect(empty.dise).toBeNull();
    expect(empty.dashboard.latestReport).toBeNull();
    expect(empty.dashboard.pendingRulings.total).toBe(0);
  });
});

describe("loadKb", () => {
  it("loads every markdown file through the store", async () => {
    const store = createMemoryStore(fixtureKb);
    const model = await loadKb(store);
    expect(model.docs).toHaveLength(Object.keys(fixtureKb).length);
    expect(model.dashboard.pendingRulings.high).toBe(2);
  });

  it("skips non-markdown entries returned by a store", async () => {
    const store: KbStore = {
      kind: "memory",
      list: async () => [
        { path: "self/底色.md", mtime: 1 },
        { path: "inbox/.gitkeep" },
        { path: "录音.mp3" },
      ],
      read: async (path) => {
        if (path !== "self/底色.md") throw new Error(`unexpected read ${path}`);
        return "# 底色\n";
      },
      write: async () => undefined,
    };
    const model = await loadKb(store);
    expect(model.docs.map((d) => [d.path, d.mtime])).toEqual([["self/底色.md", 1]]);
  });

  it("reports which file failed to load", async () => {
    const store: KbStore = {
      kind: "memory",
      list: async () => [{ path: "a.md" }],
      read: async () => {
        throw new Error("boom");
      },
      write: async () => undefined,
    };
    await expect(loadKb(store)).rejects.toThrow(/a\.md.*boom/);
  });
});
