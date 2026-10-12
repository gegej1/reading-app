// Smoke tests: every screen renders from the synthetic KB (server-side render,
// no browser needed) and shows the key facts the brief asks for.

import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router";
import { buildKbModel } from "../domain";
import { fixtureKb } from "../domain/__fixtures__";
import { createMemoryStore } from "../store/memoryStore";
import { AppRoutes } from "../App";
import { TodayScreen } from "../features/today/TodayScreen";
import { KbContext, type KbContextValue } from "./KbContext";
import { makeLinkResolver, pageRoute } from "./links";
import { NO_RUNS, RunContext, type RunContextValue } from "./RunContext";
import { RunIndicator } from "./RunIndicator";

const model = buildKbModel(Object.entries(fixtureKb).map(([path, content]) => ({ path, content })));

function contextValue(overrides: Partial<KbContextValue> = {}): KbContextValue {
  return {
    store: createMemoryStore(fixtureKb),
    model,
    writable: true,
    reloading: false,
    editing: false,
    reload: async () => undefined,
    editFile: async () => ({ changed: false, next: "", message: null }),
    resolveLink: makeLinkResolver(model),
    notice: null,
    notify: () => undefined,
    sourceLabel: "演示数据",
    sourceDetail: null,
    ...overrides,
  };
}

function render(route: string, overrides: Partial<KbContextValue> = {}) {
  return renderToStaticMarkup(
    <KbContext.Provider value={contextValue(overrides)}>
      <MemoryRouter initialEntries={[route]}>
        <AppRoutes />
      </MemoryRouter>
    </KbContext.Provider>,
  );
}

const text = (html: string) => html.replace(/<[^>]+>/g, " ").replace(/\s+/g, " ");

describe("shell", () => {
  it("has the five sections and marks the active one", () => {
    const html = render("/rulings");
    for (const label of ["今日", "底色", "裁决", "知识", "原料"]) expect(html).toContain(label);
    expect(html).toMatch(/class="spine__link is-active"[^>]*aria-current="page"[^>]*><span class="spine__label">裁决/);
  });

  it("has no settings entry and no run indicator on the web and in the demo", () => {
    const html = render("/");
    expect(html).not.toContain('href="/settings"');
    expect(html).not.toContain("run-indicator");
  });

  it("the run indicator says what a run is doing and opens the panel", () => {
    const opened: string[] = [];
    const indicator = (value: Partial<RunContextValue>) =>
      renderToStaticMarkup(
        <RunContext.Provider value={{ ...NO_RUNS, available: true, openPanel: (view) => opened.push(view ?? ""), ...value }}>
          <RunIndicator />
        </RunContext.Provider>,
      );
    // Nothing to show while no run goes on and every result was seen (今日 leads to the history).
    expect(indicator({})).toBe("");
    expect(indicator({ activeTask: "ingest", status: { phase: "running", run: null, gateAttempt: 0 } })).toMatch(/is-running[\s\S]*>入库中</);
    expect(indicator({ activeTask: "landRulings", status: { phase: "needsDecision", run: null, gateAttempt: 0 } })).toMatch(
      /is-attention[\s\S]*>待处理</,
    );
    expect(indicator({ unseen: true })).toMatch(/is-result[\s\S]*>有结果</);
    expect(renderToStaticMarkup(<RunIndicator />)).toBe("");
  });

  it("highlights the section of the page being read", () => {
    const html = render(pageRoute("self/待裁决/Q-001-周会要不要取消.md"));
    expect(html).toMatch(/is-active"[^>]*><span class="spine__label">裁决/);
  });
});

describe("今日", () => {
  it("shows counts, the latest report, the capture button and disabled future actions", () => {
    const html = renderToStaticMarkup(
      <KbContext.Provider value={contextValue()}>
        <MemoryRouter>
          <TodayScreen now={new Date(2026, 9, 12)} />
        </MemoryRouter>
      </KbContext.Provider>,
    );
    const t = text(html);
    expect(t).toContain("十月十二日");
    expect(t).toMatch(/收件箱 2 条随手记 另有 1 份回答待落裁决/);
    expect(t).toMatch(/待裁决 5 项 其中高优先级 2 项/);
    expect(t).toMatch(/待抽查 4 条/);
    expect(t).toContain("入库报告（2026-09-08 第二次）");
    // Runs only exist in the Mac app: here the buttons say so.
    expect(html).toMatch(/data-hint="只能在 Mac 上运行"[^>]*><button type="button" class="btn btn--seal" disabled="">入库 ▶/);
    expect(html).toMatch(/data-hint="只能在 Mac 上运行"[^>]*><button type="button" class="btn btn--seal" disabled="">落裁决 ▶/);
    expect(t).toContain("只能在 Mac 上运行");
    // 记一笔 works everywhere now; without a desktop shortcut it shows no key.
    expect(html).toMatch(/<button type="button" class="btn btn--quiet today__capture" title="随手记一笔，直接进收件箱">✎ 记一笔<\/button>/);
    expect(html).not.toContain("today__notice");
    expect(html).toContain('href="/rulings?tab=audit"');
    // 2026-10-12 picks a deterministic entry.
    expect(html).toMatch(/class="letter__id">D0\d</);
  });
});

describe("底色", () => {
  it("renders every entry with quote, status and seal", () => {
    const html = render("/dise");
    expect(html.match(/<article id="dise-D0\d"/g)).toHaveLength(6);
    expect(html.match(/role="img" aria-label="已确认"/g)).toHaveLength(2);
    expect(html).toContain("已确认 ✅");
    expect(html).toContain("草稿");
    expect(html).toContain("先确认能退回来，再决定跑多快。");
    expect(html).toContain("盖章确认");
    expect(html).toContain("取消确认");
    expect(html).toContain('class="wikilink wikilink--broken"');
  });

  it("shows AI drafts distinctly and offers to confirm them", () => {
    const html = render("/dise");
    const d04 = /<article id="dise-D04"[^>]*class="([^"]*)"[\s\S]*?<\/article>/.exec(html);
    expect(d04?.[1]).toContain("is-draft");
    const card = d04?.[0] ?? "";
    expect(card).toContain('<span class="entry__tag" aria-hidden="true">草稿</span>');
    expect(card).toContain("AI 草稿 · 待你确认");
    expect(card).toContain("确认草稿");
    expect(card).toContain("确认后去掉草稿标记");
    // An entry that is simply not confirmed yet is no draft.
    const d02 = /<article id="dise-D02"[\s\S]*?<\/article>/.exec(html)?.[0] ?? "";
    expect(d02).not.toContain("is-draft");
    expect(d02).toContain("未确认");
    expect(d02).toContain("盖章确认");
    expect(text(html)).toContain("AI 草稿 1 条待你确认");
  });

  it("disables the toggle in the read-only preview", () => {
    const html = render("/dise", { writable: false, sourceLabel: "开发预览 · 只读" });
    expect(html.match(/<button type="button" class="btn btn--small[^"]*" disabled=""/g)).toHaveLength(6);
    expect(html).toContain("只读预览");
  });

  it("disables every confirm button while a run holds the knowledge base, and says why", () => {
    const html = renderToStaticMarkup(
      <KbContext.Provider value={contextValue()}>
        <RunContext.Provider value={{ ...NO_RUNS, available: true, editBlocked: "正在入库，结束后再改" }}>
          <MemoryRouter initialEntries={["/dise"]}>
            <AppRoutes />
          </MemoryRouter>
        </RunContext.Provider>
      </KbContext.Provider>,
    );
    expect(html.match(/<button type="button" class="btn btn--small[^"]*" disabled=""/g)).toHaveLength(6);
    expect(html.match(/data-hint="正在入库，结束后再改"/g)).toHaveLength(6);
  });

  it("disables every confirm button while an edit is being written", () => {
    const html = render("/dise", { editing: true });
    expect(html.match(/<button type="button" class="btn btn--small[^"]*" disabled=""/g)).toHaveLength(6);
    expect(render("/dise").match(/disabled=""/g) ?? []).toHaveLength(0);
  });
});

describe("裁决", () => {
  it("lists pending rulings by priority, then id", () => {
    const html = render("/rulings");
    const ids = [...html.matchAll(/class="ruling-row__id">(Q-\d+)</g)].map((m) => m[1]);
    expect(ids).toEqual(["Q-001", "Q-010", "Q-002", "Q-003", "Q-004"]);
    expect(html).not.toContain("Q-005");
  });

  it("marks the questions that were answered and offers 落裁决 (only on the Mac)", () => {
    const html = render("/rulings");
    const t = text(html);
    expect(t).toContain("已回答 1 份，待落裁决");
    const q001 = /<a [^>]*class="ruling-row"[^>]*>(?:(?!<\/a>)[\s\S])*Q-001(?:(?!<\/a>)[\s\S])*<\/a>/.exec(html)?.[0] ?? "";
    expect(q001).toContain("已回答，待落裁决");
    const q002 = /<a [^>]*class="ruling-row"[^>]*>(?:(?!<\/a>)[\s\S])*Q-002(?:(?!<\/a>)[\s\S])*<\/a>/.exec(html)?.[0] ?? "";
    expect(q002).not.toContain("已回答");
    expect(html).toMatch(/<button type="button" class="btn btn--seal" disabled="">落裁决 ▶/);
    expect(t).not.toContain("现在只能查看");
  });

  it("shows the audit checklist with a checkbox and 不认可 for every numbered item", () => {
    const html = render("/rulings?tab=audit");
    expect(html.match(/<li class="audit-row( is-checked)?">/g)?.length).toBe(6);
    expect(text(html)).toContain("I-07");
    expect(html).toContain("已认可");
    expect(html).not.toMatch(/<input/);
    expect(html.match(/role="checkbox"/g)).toHaveLength(6);
    expect(html.match(/aria-checked="true"/g)).toHaveLength(2);
    // Checked items and the one without an id cannot be rejected.
    expect(html.match(/>不认可<\/button>/g)).toHaveLength(3);
    expect(html).toContain('data-hint="这一条没有编号，没法勾选"');
  });

  it("disables ticking while a run holds the knowledge base, but not rejecting", () => {
    const html = renderToStaticMarkup(
      <KbContext.Provider value={contextValue()}>
        <RunContext.Provider value={{ ...NO_RUNS, available: true, editBlocked: "正在入库，结束后再改" }}>
          <MemoryRouter initialEntries={["/rulings?tab=audit"]}>
            <AppRoutes />
          </MemoryRouter>
        </RunContext.Provider>
      </KbContext.Provider>,
    );
    expect(html.match(/data-hint="正在入库，结束后再改"/g)).toHaveLength(6);
    expect(html.match(/role="checkbox"[^>]*disabled=""/g)).toHaveLength(6);
    expect(html.match(/>不认可<\/button>/g)).toHaveLength(3);
  });
});

describe("知识", () => {
  it("shows 书 first, then 判例, the other wiki folders and 体系", () => {
    const html = render("/knowledge");
    const groups = [...html.matchAll(/class="catalog__title">([^<]+)</g)].map((m) => m[1]);
    expect(groups).toEqual(["书", "判例", "概念", "人物与公司", "体系"]);
    expect(text(html)).toContain("书、判例、概念、人物与公司、体系，共 9 页");
    expect(html).toContain("写下来的过程，就是把模糊的想法变清楚的过程。");
  });

  it("lists only the groups that exist in the subtitle", () => {
    const small = buildKbModel([
      { path: "wiki/概念/甲.md", content: "# 甲\n" },
      { path: "wiki/工具/乙.md", content: "# 乙\n" },
    ]);
    const t = text(render("/knowledge", { model: small, resolveLink: makeLinkResolver(small) }));
    expect(t).toContain("概念、工具，共 2 页");
    expect(t).not.toContain("判例、");
  });

  it("searches titles and full text from the URL query", () => {
    const html = render(`/knowledge?q=${encodeURIComponent("重复确认")}`);
    expect(text(html)).toContain("找到 1 页");
    expect(html).toContain('<mark class="hl">重复确认</mark>');
  });
});

describe("copy", () => {
  it("never shows milestone names, not even in tooltips", () => {
    for (const route of ["/", "/dise", "/rulings", "/rulings?tab=audit", "/knowledge", "/raw"]) {
      const html = route === "/" ? renderToStaticMarkup(
        <KbContext.Provider value={contextValue()}>
          <MemoryRouter>
            <TodayScreen now={new Date(2026, 9, 12)} />
          </MemoryRouter>
        </KbContext.Provider>,
      ) : render(route);
      const visible = `${text(html)} ${[...html.matchAll(/(?:title|data-hint|aria-label)="([^"]*)"/g)].map((m) => m[1]).join(" ")}`;
      expect(visible, route).not.toMatch(/(?<![A-Za-z])M[0-9](?![0-9.])/);
    }
  });
});

describe("answering", () => {
  it("puts an answer box under a pending question, with the answers already given", () => {
    const html = render(pageRoute("self/待裁决/Q-001-周会要不要取消.md"));
    const t = text(html);
    expect(html).toContain('<section class="answer" aria-label="回答 Q-001">');
    expect(t).toContain("已回答，待落裁决");
    expect(t).toContain("周会保留，但只开三十分钟。");
    expect(t).toContain("9 月 12 日 22:10");
    expect(html).toContain('aria-label="回答 Q-001"');
    expect(t).toContain("交上回答");
    expect(t).toContain("再补一句");
    // An unanswered question starts with an empty box.
    const q002 = render(pageRoute("self/待裁决/Q-002-外包的边界.md"));
    expect(q002).toContain('aria-label="回答 Q-002"');
    expect(text(q002)).not.toContain("已回答，待落裁决");
    // A decided question gets none.
    expect(render(pageRoute("self/待裁决/Q-005-休息日的消息.md"))).not.toContain('class="answer"');
  });

  it("shows the audit checklist of a report page as controls, the rest as Markdown", () => {
    const html = render(pageRoute("reports/2026-09-08-入库报告-2.md"));
    expect(html).toContain('<section class="report-audit" aria-label="待你抽查：AI 按判例做的裁决">');
    expect(html.match(/role="checkbox"/g)).toHaveLength(3);
    expect(text(html)).toContain("其他清单（不是抽查项）");
    expect(text(html)).toContain("I-99 这一条不在待抽查小节里");
    expect(text(html)).toContain("结果：");
  });
});

describe("page view", () => {
  it("shows frontmatter chips, rendered Markdown with wikilinks and backlinks", () => {
    const html = render(pageRoute("wiki/概念/写作即思考.md"));
    expect(html).toContain('<h1 class="page__title">写作即思考</h1>');
    expect(html).toMatch(/<dt>标签<\/dt><dd>做事、写作<\/dd>/);
    expect(html).toContain('href="#/dise"');
    // 底色, 信任的成本, 示例之书, the first report and index link here.
    expect(text(html)).toMatch(/引用了这一页的地方 5/);
    // The H1 is shown once, in the header.
    expect(html.match(/写作即思考<\/h1>/g)).toHaveLength(1);
  });

  it("renders raw sources", () => {
    const html = render("/raw");
    expect(text(html)).toContain("待入库的随手记 2");
    expect(text(html)).toContain("收件箱 1");
    expect(text(html)).toContain("对话 1");
  });

  it("handles unknown pages and routes", () => {
    expect(render(pageRoute("wiki/没有这一页.md"))).toContain("找不到「wiki/没有这一页.md」");
    expect(render("/nope")).toContain("这一页不存在");
  });

  it("opens same-named files by path and lists the other ones", () => {
    const twins = buildKbModel([
      { path: "wiki/书/README.md", content: "# 书的说明\n\n关于书。\n" },
      { path: "wiki/概念/README.md", content: "# 概念的说明\n\n关于概念，另见 [[wiki/书/README]]。\n" },
    ]);
    const html = render(pageRoute("wiki/概念/README.md"), {
      model: twins,
      resolveLink: makeLinkResolver(twins),
    });
    expect(html).toContain('<h1 class="page__title">概念的说明</h1>');
    expect(html).not.toContain("书的说明</h1>");
    expect(text(html)).toContain("同名文件");
    expect(html).toContain(`href="${pageRoute("wiki/书/README.md")}"`);
    // The path-qualified wikilink points at the other file, not the first README.
    expect(html).toContain(`href="#${pageRoute("wiki/书/README.md")}"`);

    const book = render(pageRoute("wiki/书/README.md"), {
      model: twins,
      resolveLink: makeLinkResolver(twins),
    });
    expect(book).toContain('<h1 class="page__title">书的说明</h1>');
    expect(book).toContain(`href="${pageRoute("wiki/概念/README.md")}"`);
  });

  it("does not show the same-name hint for unique names", () => {
    expect(text(render(pageRoute("wiki/概念/写作即思考.md")))).not.toContain("同名文件");
  });
});
