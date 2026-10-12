// The iPhone export may only carry the 底色 layer. These tests put
// distinctive markers into every place that must stay on the Mac (raw/,
// wiki/, systems/, inbox/, index.md, log.md, the rest of self/, the parts of
// a report that are not about rulings) and check that none of them reaches
// any exported byte; and that the export built from all files is exactly the
// export built from the allowed files alone.

import { describe, expect, it } from "vitest";
import { buildKbModel, pickDailyEntry, type KbFile } from "../domain";
import { fixtureKb } from "../domain/__fixtures__";
import { buildPhoneExport, exportModel, rulingListText } from "./phoneExport";
import { isExportSource, PAGE_FILES } from "./sources";

const NOW = new Date(2026, 9, 12, 9, 30);

/** Files that must never leave the Mac, each with its own marker. */
const FORBIDDEN: Record<string, string> = {
  "raw/inbox/2026-10-01-收件箱.md": "# 收件箱\n\nRAWMARK-inbox-7f3a 自评：我觉得自己 RAWMARK-self-eval\n",
  "raw/对话/2026-10-01-规划.md": "公司计划 RAWMARK-plan-91c2\n",
  "wiki/概念/写作即思考.md": "# 写作即思考\n\nWIKIMARK-concept-55e1\n",
  "wiki/人物与公司/秘密公司.md": "# 秘密公司\n\nWIKIMARK-company-0b7d [[P-001-速度与退路]]\n",
  "systems/周节奏.md": "# 周节奏\n\nSYSMARK-week-31aa\n",
  "inbox/2026-10-12T09-00-00-iphone.md": "---\n来源: iphone\n---\n\nINBOXMARK-capture-c0de\n",
  "inbox/裁决/Q-001-2026-10-12T09-00-00-mac.md": "---\n对象: Q-001\n---\n\nINBOXMARK-answer-a11e\n",
  "index.md": "# 目录\n\nINDEXMARK-d00d\n",
  "log.md": "# 日志\n\nLOGMARK-beef\n",
  "AGENTS.md": "# 规则\n\nAGENTSMARK-face\n",
  "self/自评.md": "# 自评\n\nSELFMARK-assessment-1234\n",
  "self/决策日志/2026-10.md": "# 决策日志\n\nSELFMARK-log-5678\n",
  "README.md": "READMEMARK-9999\n",
};

/** The newest report: its sections about rulings travel, the rest does not. */
const LATEST_REPORT: KbFile = {
  path: "reports/2026-10-11-入库报告.md",
  content: [
    "---",
    "类型: 入库报告",
    "---",
    "",
    "# 入库报告（2026-10-11）",
    "",
    "> 原文摘录 REPORTMARK-preamble-1111",
    "",
    "- 结果：自动入库 2 条｜按判例裁决 1 条（待你抽查）",
    "",
    "## 待你抽查：AI 按判例做的裁决",
    "",
    "- [ ] I-21 每周只开一次会 ↔ \"先留退路\"：兼容（依据 [[P-001-速度与退路]]）",
    "",
    "## 交给你裁决",
    "",
    "- [[Q-001-周会要不要取消]]：补了一条材料",
    "",
    "## 拆条与去向",
    "",
    "| 条目 | 摘要 |",
    "|---|---|",
    "| I-20 | REPORTMARK-split-2222 [[写作即思考]] |",
    "",
    "## 观察",
    "",
    "- REPORTMARK-observe-3333",
    "",
  ].join("\n"),
};

const MARKERS = /RAWMARK|WIKIMARK|SYSMARK|INBOXMARK|INDEXMARK|LOGMARK|AGENTSMARK|SELFMARK|READMEMARK|REPORTMARK/;

function allFiles(): KbFile[] {
  const files: KbFile[] = Object.entries(fixtureKb)
    .filter(([path]) => !(path in FORBIDDEN))
    .map(([path, content]) => ({ path, content }));
  for (const [path, content] of Object.entries(FORBIDDEN)) files.push({ path, content });
  files.push(LATEST_REPORT);
  return files;
}

const EXPORT_NAME = /^(今日|底色|判例|待裁决|待抽查|最近入库)\.html$|^(今日一条|待裁决列表)\.txt$|^待裁决\/Q-\d+\.txt$/;

describe("the iPhone export", () => {
  const files = allFiles();
  const exported = buildPhoneExport(files, NOW);
  const byName = new Map(exported.map((file) => [file.name, file.content]));

  it("reads only the 底色 layer", () => {
    expect(isExportSource("self/底色.md")).toBe(true);
    expect(isExportSource("self/判例/P-001-速度与退路.md")).toBe(true);
    expect(isExportSource("self/待裁决/Q-001-周会要不要取消.md")).toBe(true);
    expect(isExportSource("reports/2026-09-08-入库报告.md")).toBe(true);
    for (const path of [
      ...Object.keys(FORBIDDEN),
      "self/待裁决.md",
      "self/判例/旧/P-009.md",
      "reports/归档/2026-01-01-入库报告.md",
      "raw/self/底色.md",
      "self/底色.md.bak",
    ]) {
      expect(isExportSource(path), path).toBe(false);
    }
  });

  it("carries no byte from a file outside the 底色 layer", () => {
    // Make sure the markers really are in the input.
    expect(files.some((file) => MARKERS.test(file.content))).toBe(true);
    for (const file of exported) {
      const found = MARKERS.exec(file.content);
      expect(found?.[0] ?? null, `${file.name} contains ${found?.[0]}`).toBeNull();
    }
    // Exactly the same export without the forbidden files: nothing in it
    // depends on them.
    const allowedOnly = files.filter((file) => isExportSource(file.path));
    expect(buildPhoneExport(allowedOnly, NOW)).toEqual(exported);
    const withoutForbidden = files.filter((file) => !(file.path in FORBIDDEN));
    expect(buildPhoneExport(withoutForbidden, NOW)).toEqual(exported);
  });

  it("writes only the export's file names", () => {
    for (const file of exported) expect(file.name).toMatch(EXPORT_NAME);
    expect(new Set(exported.map((f) => f.name)).size).toBe(exported.length);
    for (const name of Object.values(PAGE_FILES)) expect(byName.has(name), name).toBe(true);
  });

  it("makes self-contained pages without scripts or outside resources", () => {
    for (const file of exported.filter((f) => f.name.endsWith(".html"))) {
      const html = file.content;
      expect(html.startsWith("<!doctype html>"), file.name).toBe(true);
      expect(html).not.toMatch(/<script|<iframe|<object|<embed|<link\b|\son[a-z]+=|javascript:/i);
      expect(html).not.toMatch(/\ssrc=/i);
      expect(html).toContain('http-equiv="Content-Security-Policy"');
      expect(html).toContain("default-src &#x27;none&#x27;");
      expect(html).toContain("prefers-color-scheme: dark");
      expect(html).toContain('name="viewport"');
    }
  });

  it("links only to exported pages and anchors that exist", () => {
    const ids = new Map<string, Set<string>>();
    for (const file of exported) {
      ids.set(file.name, new Set([...file.content.matchAll(/\sid="([^"]+)"/g)].map((m) => m[1] ?? "")));
    }
    for (const file of exported.filter((f) => f.name.endsWith(".html"))) {
      for (const [, href] of file.content.matchAll(/href="([^"]*)"/g)) {
        if (href === undefined || /^https?:\/\//.test(href)) continue;
        const [page, anchor] = href.split("#");
        const target = page === "" ? file.name : decodeURIComponent(page ?? "");
        expect(byName.has(target), `${file.name} -> ${href}`).toBe(true);
        if (anchor) expect(ids.get(target)?.has(decodeURIComponent(anchor)), `${file.name} -> ${href}`).toBe(true);
      }
    }
  });

  it("shows links to pages that stay on the Mac as plain text", () => {
    const dise = byName.get(PAGE_FILES.dise) ?? "";
    expect(dise).toContain("写作即思考");
    expect(dise).not.toMatch(/<a[^>]*>写作即思考/);
    // The raw conversation the 底色 cites is named, not linked.
    expect(dise).toContain("2026-09-01-示例对话");
    expect(dise).not.toMatch(/<a[^>]*>2026-09-01-示例对话/);
    expect(dise).not.toMatch(/title="/);
    // A precedent link becomes a link to the precedents page.
    expect(dise).toContain('href="判例.html#P-001"');
  });

  it("shows the 底色 with confirmations and drafts", () => {
    const dise = byName.get(PAGE_FILES.dise) ?? "";
    expect(dise).toContain('id="D01"');
    expect(dise).toMatch(/class="card is-confirmed" id="D01"/);
    expect(dise).toMatch(/class="card is-draft" id="D04"/);
    expect(dise).toContain("草稿，待你确认");
    expect(dise).toContain("已确认");
    expect(dise).not.toContain('id="D99"');
  });

  it("lists the open rulings with materials, question and answering help", () => {
    const model = exportModel(files);
    const page = byName.get(PAGE_FILES.rulings) ?? "";
    for (const ruling of model.pendingRulings) expect(page).toContain(`id="${ruling.id}"`);
    expect(page).toContain("新材料");
    expect(page).toContain("要你定");
    expect(page).toContain("答裁决");
    expect(page).not.toContain('id="Q-005"');

    const list = byName.get("待裁决列表.txt") ?? "";
    expect(list).toBe(rulingListText(model));
    const lines = list.trimEnd().split("\n");
    expect(lines.length).toBe(model.pendingRulings.length);
    for (const line of lines) expect(line).toMatch(/^Q-\d+｜\S/);
    expect(lines[0]).toBe("Q-001｜周会要不要取消");
    const question = byName.get("待裁决/Q-001.txt") ?? "";
    expect(question).toContain("Q-001 周会要不要取消");
    expect(question).toContain("要你定：周会保留还是取消？");
    expect(question).toContain("AI 建议（参考）：保留周会");
    expect(byName.has("待裁决/Q-005.txt")).toBe(false);
  });

  it("shows only the ruling sections of the latest report and the unchecked audits", () => {
    const report = byName.get(PAGE_FILES.report) ?? "";
    expect(report).toContain("待你抽查");
    expect(report).toContain("I-21");
    expect(report).toContain("交给你裁决");
    expect(report).toContain("拆条与去向、观察只在 Mac 上看");
    const audits = byName.get(PAGE_FILES.audits) ?? "";
    expect(audits).toContain("I-21");
    const model = buildKbModel(files.filter((f) => isExportSource(f.path)));
    for (const item of model.auditItems) {
      if (item.itemId) expect(audits.includes(`>${item.itemId}<`), item.itemId).toBe(!item.checked);
    }
  });

  it("puts today's principle on 今日 and into 今日一条.txt", () => {
    const model = exportModel(files);
    const entry = pickDailyEntry(model.dise?.entries ?? [], NOW);
    expect(entry).not.toBeNull();
    const daily = byName.get("今日一条.txt") ?? "";
    expect(daily.startsWith(`${entry?.id} ${entry?.title}`)).toBe(true);
    expect(daily).toContain("2026 年 10 月 12 日");
    const today = byName.get(PAGE_FILES.today) ?? "";
    expect(today).toContain("十月十二日");
    expect(today).toContain(`id="${entry?.id}"`);
    expect(today).toContain("导出于 <span class=\"num\">09:30</span>");
  });

  it("works on an empty knowledge base", () => {
    const empty = buildPhoneExport([], NOW);
    expect(empty.map((f) => f.name)).toEqual([
      "今日.html",
      "底色.html",
      "判例.html",
      "待裁决.html",
      "待抽查.html",
      "最近入库.html",
      "今日一条.txt",
      "待裁决列表.txt",
    ]);
    expect(empty.find((f) => f.name === "待裁决列表.txt")?.content).toBe("");
  });
});
