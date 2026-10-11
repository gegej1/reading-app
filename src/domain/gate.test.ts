import { describe, expect, it } from "vitest";
import example from "./__fixtures__/gate-example.json";
import {
  isAllowedPath,
  parseFinalLine,
  queueBody,
  runGate,
  type GateFile,
  type GateInput,
  type GateIssue,
  type RunTask,
} from "./gate";

// A tiny synthetic knowledge base. Everything here is invented.
const DISE = [
  "---",
  "类型: 底色",
  "---",
  "",
  "# 示例底色",
  "",
  "## 一、怎么做选择",
  "",
  "### D01 先留退路 ✅",
  "> 先确认能退回来，再决定跑多快。",
  "",
  "- 判例：[[P-001-速度与退路]]",
  "",
  "### D02 一次只做一件事",
  "> 主线只有一条。",
  "",
].join("\n");

const CAPTURE_1 = "---\n来源: mac\n时间: 2026-10-12T09:00:00+08:00\n---\n\n周五下午做一次周回顾，只看三件事。\n";
const CAPTURE_2 = "---\n来源: iphone\n时间: 2026-10-12T09:05:00+08:00\n---\n\n清单越短，越容易做完。\n第二行也要逐字保留。\n";
const ANSWER = "---\n对象: Q-004\n时间: 2026-10-12T10:00:00+08:00\n---\n\n工具先不统一，各组自己选。\n";
const AUDIT_ANSWER = "---\n对象: 抽查 2026-10-11-入库报告 I-02\n时间: 2026-10-12T10:05:00+08:00\n---\n\n这条判错了，会议少不等于做事多。\n";

const Q004 = "---\n类型: 待裁决\n编号: Q-004\n状态: 待裁决\n分类: 衍生问题\n优先级: 低\n---\n\n# Q-004 工具要不要统一\n\n## 要你定\n统一还是不统一？\n";

const BASE: Record<string, string> = {
  "index.md": "# 总目录\n\n- [[底色]]\n- [[小实验]]\n",
  "log.md": "# 操作日志\n\n## [2026-10-11] ingest | 示例\n",
  "self/底色.md": DISE,
  "self/判例/P-001-速度与退路.md": "# P-001 速度与退路\n",
  "self/待裁决.md": "# 待裁决\n\n- [[Q-004-工具要不要统一]]\n",
  "self/待裁决/Q-004-工具要不要统一.md": Q004,
  "raw/inbox/2026-10-11-收件箱.md": "# 2026-10-11 收件箱\n\n## I-01\n小实验比大计划更快。\n",
  "wiki/概念/小实验.md": "# 小实验\n\n出处：[[2026-10-11-收件箱]]\n",
  "reports/2026-10-11-入库报告.md": "# 入库报告\n\n- 结果：1 条\n",
  "inbox/2026-10-12T09-00-00-mac.md": CAPTURE_1,
  "inbox/2026-10-12T09-05-00-iphone.md": CAPTURE_2,
  "inbox/裁决/2026-10-12T10-00-00-mac.md": ANSWER,
};

const INGEST_QUEUE = ["inbox/2026-10-12T09-00-00-mac.md", "inbox/2026-10-12T09-05-00-iphone.md"];
const RULING_QUEUE = ["inbox/裁决/2026-10-12T10-00-00-mac.md"];

const TODAY_RAW = [
  "# 2026-10-12 收件箱",
  "",
  "## I-01（mac）",
  "周五下午做一次周回顾，只看三件事。",
  "",
  "## I-02（iphone）",
  "清单越短，越容易做完。",
  "第二行也要逐字保留。",
  "",
].join("\n");

const FINAL_INGEST =
  '{"任务": "入库", "条数": 2, "自动": 2, "判例裁决": 0, "待裁决": 0, "报告": "reports/2026-10-12-入库报告.md", "备注": "本次没有拉取 Notion"}';

/** What a well-behaved ingest leaves behind (queue files stay: the engine may not delete). */
function ingestAfter(): Record<string, string> {
  return {
    ...BASE,
    "raw/inbox/2026-10-12-收件箱.md": TODAY_RAW,
    "wiki/概念/小实验.md": "# 小实验\n\n出处：[[2026-10-11-收件箱]]、[[2026-10-12-收件箱]]\n",
    "reports/2026-10-12-入库报告.md": "# 入库报告（2026-10-12）\n\n- 结果：2 条\n- 本次没有拉取 Notion。\n",
    "index.md": `${BASE["index.md"]}- [[2026-10-12-入库报告]]\n`,
    "log.md": `${BASE["log.md"]}## [2026-10-12] ingest | 随手记 2 条\n`,
  };
}

const ANSWER_RAW = "# 2026-10-12 裁决\n\n## Q-004（2026-10-12T10:00:00+08:00）\n工具先不统一，各组自己选。\n";

function rulingsAfter(): Record<string, string> {
  return {
    ...BASE,
    "raw/对话/2026-10-12-裁决.md": ANSWER_RAW,
    "self/待裁决/Q-004-工具要不要统一.md": Q004.replace("状态: 待裁决", "状态: 已裁决").concat(
      "\n## 裁决\n见 [[2026-10-12-裁决]]。\n",
    ),
    "log.md": `${BASE["log.md"]}## [2026-10-12] rulings | 1 份\n`,
  };
}

interface Scenario {
  task?: RunTask;
  before?: Record<string, string>;
  after: Record<string, string>;
  queue?: string[];
  finalMessage?: string | null;
  engineFailure?: string | null;
  foreignCommits?: { hash: string; subject: string }[];
  untrackedChanged?: string[];
  opaque?: Record<string, GateFile["opaque"]>;
}

/** Builds the gate input the app would send for a before/after pair of file maps. */
function input(s: Scenario): GateInput {
  const task = s.task ?? "ingest";
  const before = s.before ?? BASE;
  const queuePaths = s.queue ?? (task === "ingest" ? INGEST_QUEUE : RULING_QUEUE);
  const paths = [...new Set([...Object.keys(before), ...Object.keys(s.after)])].sort();
  const changes: GateFile[] = [];
  const unchanged: { path: string; content: string }[] = [];
  for (const path of paths) {
    const b = before[path] ?? null;
    const a = s.after[path] ?? null;
    if (b === a) {
      if (a !== null && path.endsWith(".md")) unchanged.push({ path, content: a });
      continue;
    }
    const opaque = s.opaque?.[path] ?? null;
    changes.push({
      path,
      status: b === null ? "added" : a === null ? "deleted" : "modified",
      before: opaque ? null : b,
      after: opaque ? null : a,
      opaque,
    });
  }
  return {
    task,
    date: "2026-10-12",
    changes,
    unchanged,
    queue: queuePaths.map((path) => ({ path, content: before[path] ?? "" })),
    finalMessage:
      s.finalMessage === undefined
        ? task === "ingest"
          ? FINAL_INGEST
          : FINAL_INGEST.replace('"入库"', '"落裁决"').replace('"条数": 2', '"条数": 1').replace(/"报告": "[^"]*"/, '"报告": ""')
        : s.finalMessage,
    engineFailure: s.engineFailure ?? null,
    foreignCommits: s.foreignCommits ?? [],
    untrackedChanged: s.untrackedChanged ?? [],
  };
}

const rules = (issues: GateIssue[]) => issues.map((issue) => issue.rule);
const messages = (issues: GateIssue[]) => issues.map((issue) => issue.message).join("\n");

describe("isAllowedPath", () => {
  it("allows the five content folders and the two root pages", () => {
    for (const path of [
      "raw/inbox/2026-10-12-收件箱.md",
      "wiki/概念/新概念.md",
      "systems/周节奏.md",
      "self/判例/P-009-新判例.md",
      "reports/2026-10-12-入库报告.md",
      "index.md",
      "log.md",
    ]) {
      expect(isAllowedPath(path), path).toBe(true);
    }
  });

  it("refuses everything else", () => {
    for (const path of [
      "AGENTS.md",
      "CLAUDE.md",
      "README.md",
      ".gitignore",
      ".git/config",
      "inbox/2026-10-12T09-00-00-mac.md",
      "inbox/裁决/a.md",
      "rawx/a.md",
      "raw",
      "wiki",
      "notes/index.md",
      "self",
    ]) {
      expect(isAllowedPath(path), path).toBe(false);
    }
  });
});

describe("queueBody", () => {
  it("drops the frontmatter, normalises line endings and trims the outside", () => {
    expect(queueBody(CAPTURE_2)).toBe("清单越短，越容易做完。\n第二行也要逐字保留。");
    expect(queueBody("---\r\n来源: web\r\n---\r\n\r\n  第一行\r\n第二行  \r\n\r\n")).toBe("第一行\n第二行");
    expect(queueBody("没有 frontmatter 的一句话\n")).toBe("没有 frontmatter 的一句话");
  });
});

describe("runGate: a well-behaved run", () => {
  it("passes an ingest and lists exactly what to commit and which queue files to delete", () => {
    const report = runGate(input({ after: ingestAfter() }));
    expect(report.violations).toEqual([]);
    expect(report.warnings).toEqual([]);
    expect(report.ok).toBe(true);
    expect(report.commitPaths).toEqual([
      "index.md",
      "log.md",
      "raw/inbox/2026-10-12-收件箱.md",
      "reports/2026-10-12-入库报告.md",
      "wiki/概念/小实验.md",
    ]);
    expect(report.deleteQueuePaths).toEqual(INGEST_QUEUE);
    expect(report.restoreQueuePaths).toEqual([]);
    expect(report.final).toMatchObject({ 任务: "入库", 条数: 2 });
    expect(report.summary).toContain("通过");
  });

  it("passes a land-rulings run", () => {
    const report = runGate(input({ task: "landRulings", after: rulingsAfter() }));
    expect(report.violations).toEqual([]);
    expect(report.ok).toBe(true);
    expect(report.deleteQueuePaths).toEqual(RULING_QUEUE);
    expect(report.commitPaths).toEqual(["log.md", "raw/对话/2026-10-12-裁决.md", "self/待裁决/Q-004-工具要不要统一.md"]);
  });
});

describe("runGate: allowed paths", () => {
  it("flags changes outside the allowed folders, naming the file", () => {
    for (const path of ["README.md", "AGENTS.md", "notes/new.md", ".gitignore"]) {
      const after = { ...ingestAfter(), [path]: "改了\n" };
      const report = runGate(input({ after }));
      expect(report.ok, path).toBe(false);
      expect(rules(report.violations), path).toEqual(["paths"]);
      expect(report.violations[0]?.path).toBe(path);
      expect(report.violations[0]?.message).toContain(path);
    }
  });

  it("flags an engine that edits or adds inbox files, but not one that deletes a processed queue file", () => {
    const edited = { ...ingestAfter(), "inbox/2026-10-12T09-00-00-mac.md": `${CAPTURE_1}多了一行\n` };
    const report = runGate(input({ after: edited }));
    expect(rules(report.violations)).toEqual(["paths"]);
    expect(report.violations[0]?.path).toBe("inbox/2026-10-12T09-00-00-mac.md");

    const added = { ...ingestAfter(), "inbox/新的.md": "不该有\n" };
    expect(rules(runGate(input({ after: added })).violations)).toEqual(["paths"]);

    const deleted = ingestAfter();
    delete deleted["inbox/2026-10-12T09-00-00-mac.md"];
    const ok = runGate(input({ after: deleted }));
    expect(ok.violations).toEqual([]);
    expect(ok.deleteQueuePaths).toEqual(INGEST_QUEUE);
    expect(ok.commitPaths).not.toContain("inbox/2026-10-12T09-00-00-mac.md");
  });

  it("refuses symlinks", () => {
    const after = { ...ingestAfter(), "wiki/概念/链接.md": "->" };
    const report = runGate(input({ after, opaque: { "wiki/概念/链接.md": "symlink" } }));
    expect(rules(report.violations)).toEqual(["fileKind"]);
  });
});

describe("runGate: raw/ only grows at the end", () => {
  it("accepts appending to an existing raw file", () => {
    const after = { ...ingestAfter() };
    after["raw/inbox/2026-10-11-收件箱.md"] = `${BASE["raw/inbox/2026-10-11-收件箱.md"]}\n## I-02\n追加的一条。\n`;
    expect(runGate(input({ after })).violations).toEqual([]);
  });

  it("flags a raw file rewritten in the middle", () => {
    const after = { ...ingestAfter() };
    after["raw/inbox/2026-10-11-收件箱.md"] = "# 2026-10-11 收件箱\n\n## I-01\n小实验比大计划更快找到答案。\n";
    const report = runGate(input({ after }));
    expect(rules(report.violations)).toEqual(["rawAppendOnly"]);
    expect(report.violations[0]?.message).toContain("raw/inbox/2026-10-11-收件箱.md");
  });

  it("flags deleting and renaming raw files", () => {
    const deleted = ingestAfter();
    delete deleted["raw/inbox/2026-10-11-收件箱.md"];
    // The deleted raw file was also a link target, so a link breaks too.
    expect(rules(runGate(input({ after: deleted })).violations)).toContain("rawAppendOnly");

    const renamed = ingestAfter();
    renamed["raw/inbox/2026-10-11-旧收件箱.md"] = renamed["raw/inbox/2026-10-11-收件箱.md"] ?? "";
    delete renamed["raw/inbox/2026-10-11-收件箱.md"];
    const report = runGate(input({ after: renamed }));
    expect(report.violations.filter((v) => v.rule === "rawAppendOnly").map((v) => v.path)).toEqual([
      "raw/inbox/2026-10-11-收件箱.md",
    ]);
  });

  it("cannot check binary raw changes and says so", () => {
    const after = { ...ingestAfter(), "raw/inbox/2026-10-11-收件箱.md": "binary" };
    const report = runGate(input({ after, opaque: { "raw/inbox/2026-10-11-收件箱.md": "binary" } }));
    expect(rules(report.violations)).toEqual(["rawAppendOnly"]);
  });
});

describe("runGate: queue items are copied verbatim", () => {
  it("flags a capture whose body is not in raw/ and keeps it", () => {
    const after = { ...ingestAfter(), "raw/inbox/2026-10-12-收件箱.md": TODAY_RAW.replace("第二行也要逐字保留。", "第二行改了几个字。") };
    const report = runGate(input({ after }));
    expect(rules(report.violations)).toEqual(["queueVerbatim"]);
    expect(report.violations[0]?.path).toBe("inbox/2026-10-12T09-05-00-iphone.md");
    expect(report.deleteQueuePaths).toEqual(["inbox/2026-10-12T09-00-00-mac.md"]);
    expect(report.restoreQueuePaths).toEqual([]);
  });

  it("restores a capture the engine deleted without copying it", () => {
    const after: Record<string, string> = { ...ingestAfter(), "raw/inbox/2026-10-12-收件箱.md": "# 2026-10-12 收件箱\n" };
    delete after["inbox/2026-10-12T09-00-00-mac.md"];
    delete after["inbox/2026-10-12T09-05-00-iphone.md"];
    const report = runGate(input({ after }));
    expect(rules(report.violations)).toEqual(["queueVerbatim", "queueVerbatim"]);
    expect(messages(report.violations)).toContain("删掉");
    expect(report.deleteQueuePaths).toEqual([]);
    expect(report.restoreQueuePaths).toEqual(INGEST_QUEUE);
  });

  it("deletes a queue file the engine deleted after copying it", () => {
    const after = ingestAfter();
    delete after["inbox/2026-10-12T09-05-00-iphone.md"];
    const report = runGate(input({ after }));
    expect(report.ok).toBe(true);
    expect(report.deleteQueuePaths).toEqual(INGEST_QUEUE);
  });

  it("restores a queue file the engine edited when its original body is not in raw/", () => {
    const after: Record<string, string> = { ...ingestAfter(), "raw/inbox/2026-10-12-收件箱.md": "# 只有标题\n" };
    after["inbox/2026-10-12T09-00-00-mac.md"] = `${CAPTURE_1}改过\n`;
    const report = runGate(input({ after }));
    expect(report.restoreQueuePaths).toEqual(["inbox/2026-10-12T09-00-00-mac.md"]);
  });

  it("finds a body with CRLF line endings, and one already in an unchanged raw file", () => {
    const before = { ...BASE, "inbox/2026-10-12T09-05-00-iphone.md": CAPTURE_2.replace(/\n/g, "\r\n") };
    const crlf = runGate(input({ before, after: { ...ingestAfter(), "inbox/2026-10-12T09-05-00-iphone.md": before["inbox/2026-10-12T09-05-00-iphone.md"] ?? "" } }));
    expect(crlf.violations).toEqual([]);

    const already = { ...BASE, "raw/inbox/2026-10-11-收件箱.md": `${BASE["raw/inbox/2026-10-11-收件箱.md"]}${TODAY_RAW}` };
    const after: Record<string, string> = {
      ...ingestAfter(),
      "raw/inbox/2026-10-11-收件箱.md": already["raw/inbox/2026-10-11-收件箱.md"] ?? "",
      "wiki/概念/小实验.md": BASE["wiki/概念/小实验.md"] ?? "",
    };
    delete after["raw/inbox/2026-10-12-收件箱.md"];
    const report = runGate(input({ before: already, after }));
    expect(report.violations).toEqual([]);
    expect(report.deleteQueuePaths).toEqual(INGEST_QUEUE);
  });

  it("ignores a capture made while the engine ran: it is neither a change nor part of the queue", () => {
    const after = { ...ingestAfter(), "inbox/2026-10-12T09-30-00-mac.md": "---\n来源: mac\n---\n\n运行中记下的一条。\n" };
    const scenario = input({ after });
    // The app commits a capture on its own, so it reaches the gate as an
    // unchanged file, not as a change of this run.
    scenario.changes = scenario.changes.filter((c) => c.path !== "inbox/2026-10-12T09-30-00-mac.md");
    scenario.unchanged.push({ path: "inbox/2026-10-12T09-30-00-mac.md", content: after["inbox/2026-10-12T09-30-00-mac.md"] ?? "" });
    const report = runGate(scenario);
    expect(report.ok).toBe(true);
    expect(report.deleteQueuePaths).toEqual(INGEST_QUEUE);
    expect(report.commitPaths).not.toContain("inbox/2026-10-12T09-30-00-mac.md");
  });
});

describe("runGate: confirmed 底色 entries", () => {
  const withDise = (dise: string) => ({ ...ingestAfter(), "self/底色.md": dise });

  it("accepts a new draft entry and changes to unconfirmed entries", () => {
    const dise = `${DISE.replace("> 主线只有一条。", "> 主线只有一条，其余是配角。")}\n### D03 新草稿（草稿，待你确认）\n> 原话。\n`;
    expect(runGate(input({ after: withDise(dise) })).violations).toEqual([]);
  });

  it("flags any change to a confirmed entry's text", () => {
    const report = runGate(input({ after: withDise(DISE.replace("再决定跑多快", "再决定跑得多快")) }));
    expect(rules(report.violations)).toEqual(["confirmedDise"]);
    expect(report.violations[0]?.message).toContain("D01");
    expect(report.violations[0]?.path).toBe("self/底色.md");
  });

  it("flags a removed and an added check mark", () => {
    const removed = runGate(input({ after: withDise(DISE.replace("### D01 先留退路 ✅", "### D01 先留退路")) }));
    expect(rules(removed.violations)).toEqual(["confirmedDise"]);
    const added = runGate(input({ after: withDise(DISE.replace("### D02 一次只做一件事", "### D02 一次只做一件事 ✅")) }));
    expect(rules(added.violations)).toEqual(["confirmedDise"]);
    expect(added.violations[0]?.message).toContain("D02");
  });

  it("flags deleting the 底色 file when it has confirmed entries", () => {
    const after = ingestAfter();
    delete after["self/底色.md"];
    expect(rules(runGate(input({ after })).violations)).toContain("confirmedDise");
  });
});

describe("runGate: wikilinks", () => {
  it("flags a new broken link and names where it is", () => {
    const after = { ...ingestAfter(), "wiki/概念/新概念.md": "# 新概念\n\n见 [[不存在的页面]]。\n" };
    const report = runGate(input({ after }));
    expect(rules(report.violations)).toEqual(["brokenLinks"]);
    expect(report.violations[0]?.path).toBe("wiki/概念/新概念.md");
    expect(report.violations[0]?.message).toContain("不存在的页面");
  });

  it("does not count broken links that were there before, or links the run fixed", () => {
    const before = { ...BASE, "wiki/概念/旧的.md": "# 旧的\n\n[[早就断了]] [[以后会有]]\n" };
    const after = { ...ingestAfter(), "wiki/概念/旧的.md": before["wiki/概念/旧的.md"] ?? "", "wiki/概念/以后会有.md": "# 以后会有\n" };
    expect(runGate(input({ before, after })).violations).toEqual([]);
  });

  it("flags links broken by deleting their target", () => {
    const after = ingestAfter();
    delete after["self/判例/P-001-速度与退路.md"];
    const report = runGate(input({ after }));
    expect(rules(report.violations)).toEqual(["brokenLinks"]);
    expect(report.violations[0]?.path).toBe("self/底色.md");
  });

  it("only warns about a broken link copied verbatim from a capture into raw/", () => {
    const capture = "---\n来源: mac\n---\n\n想写一页 [[还没有的页面]]。\n";
    const before = { ...BASE, "inbox/2026-10-12T09-00-00-mac.md": capture };
    const after = {
      ...ingestAfter(),
      "inbox/2026-10-12T09-00-00-mac.md": capture,
      "raw/inbox/2026-10-12-收件箱.md": `${TODAY_RAW}\n## I-03\n想写一页 [[还没有的页面]]。\n`,
    };
    const finalMessage = FINAL_INGEST.replace('"条数": 2', '"条数": 1');
    const report = runGate(input({ before, after, queue: ["inbox/2026-10-12T09-00-00-mac.md"], finalMessage }));
    expect(report.violations).toEqual([]);
    expect(rules(report.warnings)).toEqual(["brokenLinks"]);
  });
});

describe("runGate: ingest writes a report", () => {
  it("flags an ingest without a new or extended report", () => {
    const after = ingestAfter();
    delete after["reports/2026-10-12-入库报告.md"];
    after["index.md"] = BASE["index.md"] ?? "";
    const report = runGate(input({ after }));
    expect(rules(report.violations)).toEqual(["report"]);
  });

  it("accepts extending an existing report of the same day", () => {
    const before = { ...BASE, "reports/2026-10-12-入库报告.md": "# 入库报告\n\n- 结果：1 条\n" };
    const after = { ...ingestAfter(), "reports/2026-10-12-入库报告.md": `${before["reports/2026-10-12-入库报告.md"]}\n## 第二次\n- 结果：2 条\n` };
    expect(runGate(input({ before, after })).violations).toEqual([]);
  });

  it("warns when the final line names a report that was not written", () => {
    const report = runGate(input({ after: ingestAfter(), finalMessage: FINAL_INGEST.replace("2026-10-12-入库报告", "2026-10-12-入库报告-2") }));
    expect(report.ok).toBe(true);
    expect(rules(report.warnings)).toEqual(["finalLine"]);
  });
});

describe("runGate: land-rulings marks the answered questions", () => {
  it("flags a Q file that still says 待裁决", () => {
    const after = rulingsAfter();
    after["self/待裁决/Q-004-工具要不要统一.md"] = Q004;
    const report = runGate(input({ task: "landRulings", after }));
    expect(rules(report.violations)).toEqual(["rulingsAnswered"]);
    expect(report.violations[0]?.message).toContain("Q-004");
  });

  it("flags an answer whose Q file does not exist", () => {
    const before = { ...BASE, "inbox/裁决/2026-10-12T10-00-00-mac.md": ANSWER.replace("Q-004", "Q-099") };
    const after = { ...rulingsAfter(), "inbox/裁决/2026-10-12T10-00-00-mac.md": before["inbox/裁决/2026-10-12T10-00-00-mac.md"] ?? "" };
    const report = runGate(input({ task: "landRulings", before, after }));
    expect(rules(report.violations)).toEqual(["rulingsAnswered"]);
    expect(report.violations[0]?.message).toContain("Q-099");
  });

  it("expects a new Q file for a rejected audit item", () => {
    const before = { ...BASE, "inbox/裁决/2026-10-12T10-05-00-mac.md": AUDIT_ANSWER };
    const queue = ["inbox/裁决/2026-10-12T10-05-00-mac.md"];
    const raw = `# 2026-10-12 裁决\n\n## 抽查\n${queueBody(AUDIT_ANSWER)}\n`;
    const without = { ...before, "raw/对话/2026-10-12-裁决.md": raw };
    expect(rules(runGate(input({ task: "landRulings", before, after: without, queue })).violations)).toEqual(["rulingsAnswered"]);

    const withQ = {
      ...without,
      "self/待裁决/Q-005-会议和做事.md": "---\n类型: 待裁决\n编号: Q-005\n状态: 待裁决\n分类: 抽查驳回\n优先级: 中\n---\n\n# Q-005 会议和做事\n",
    };
    const report = runGate(input({ task: "landRulings", before, after: withQ, queue }));
    expect(report.violations).toEqual([]);
    expect(report.deleteQueuePaths).toEqual(queue);
  });

  it("warns about an answer that does not say what it answers", () => {
    const before = { ...BASE, "inbox/裁决/2026-10-12T10-00-00-mac.md": "---\n时间: 2026-10-12\n---\n\n工具先不统一，各组自己选。\n" };
    const after = { ...rulingsAfter(), "inbox/裁决/2026-10-12T10-00-00-mac.md": before["inbox/裁决/2026-10-12T10-00-00-mac.md"] ?? "" };
    const report = runGate(input({ task: "landRulings", before, after }));
    expect(report.violations).toEqual([]);
    expect(rules(report.warnings)).toEqual(["rulingsAnswered"]);
  });
});

describe("runGate: the engine itself", () => {
  it("flags commits the engine made on its own", () => {
    const report = runGate(input({ after: ingestAfter(), foreignCommits: [{ hash: "0123456789abcdef", subject: "入库" }] }));
    expect(rules(report.violations)).toEqual(["engineCommits"]);
    expect(report.violations[0]?.message).toContain("0123456789");
  });

  it("flags an engine that did not finish normally", () => {
    const report = runGate(input({ after: ingestAfter(), engineFailure: "超过 30 分钟没有结束，已停止" }));
    expect(rules(report.violations)).toEqual(["engine"]);
    expect(report.violations[0]?.message).toContain("超过 30 分钟");
  });

  it("flags changes to files that were never in git", () => {
    const report = runGate(input({ after: ingestAfter(), untrackedChanged: ["草稿.md"] }));
    expect(rules(report.violations)).toEqual(["untracked"]);
    expect(report.violations[0]?.path).toBe("草稿.md");
  });
});

describe("runGate: the final JSON line", () => {
  it("only warns when it is missing or malformed", () => {
    for (const finalMessage of [null, "", "全部完成了。", '{"任务": "入库", ']) {
      const report = runGate(input({ after: ingestAfter(), finalMessage }));
      expect(report.ok, String(finalMessage)).toBe(true);
      expect(rules(report.warnings), String(finalMessage)).toEqual(["finalLine"]);
      expect(report.final).toBeNull();
    }
  });

  it("warns when it names the wrong task or count", () => {
    const report = runGate(input({ after: ingestAfter(), finalMessage: FINAL_INGEST.replace('"条数": 2', '"条数": 3') }));
    expect(report.ok).toBe(true);
    expect(rules(report.warnings)).toEqual(["finalLine"]);
  });
});

describe("parseFinalLine", () => {
  it("reads the last line, also inside a code fence or after other text", () => {
    expect(parseFinalLine(FINAL_INGEST)).toMatchObject({ 任务: "入库" });
    expect(parseFinalLine(`做完了。\n\n${FINAL_INGEST}\n`)).toMatchObject({ 条数: 2 });
    expect(parseFinalLine(`\`\`\`json\n${FINAL_INGEST}\n\`\`\``)).toMatchObject({ 自动: 2 });
    expect(parseFinalLine("[1, 2]")).toBeNull();
    expect(parseFinalLine("没有 JSON")).toBeNull();
  });
});

describe("runGate: everything together", () => {
  it("lists every violation, so one look shows all problems", () => {
    const after: Record<string, string> = { ...ingestAfter(), "README.md": "改了\n" };
    after["raw/inbox/2026-10-11-收件箱.md"] = "重写\n";
    after["raw/inbox/2026-10-12-收件箱.md"] = "# 空\n";
    const report = runGate(input({ after, finalMessage: null }));
    expect(report.ok).toBe(false);
    expect(new Set(rules(report.violations))).toEqual(new Set(["paths", "rawAppendOnly", "queueVerbatim"]));
    expect(report.summary).toContain("没通过");
    expect(report.summary).toContain(String(report.violations.length));
  });
});

describe("the example shared with the desktop app", () => {
  // src-tauri/src/run/gate.rs reads the same file: the input must have
  // exactly the fields the app sends, the report exactly what it reads.
  it("gets exactly the report the app expects for the input the app sends", () => {
    expect(runGate(example.input as GateInput)).toEqual(example.report);
  });
});
