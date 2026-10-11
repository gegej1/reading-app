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

  it("highlights the section of the page being read", () => {
    const html = render(pageRoute("self/待裁决/Q-001-周会要不要取消.md"));
    expect(html).toMatch(/is-active"[^>]*><span class="spine__label">裁决/);
  });
});

describe("今日", () => {
  it("shows counts, the latest report and disabled future actions", () => {
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
    expect(html).toMatch(/data-hint="M3[^"]*"[^>]*><button type="button" class="btn btn--seal" disabled="">入库 ▶/);
    expect(html).toMatch(/data-hint="M2[^"]*"[^>]*><button type="button" class="btn btn--quiet" disabled="">✎ 记一笔/);
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

  it("disables the toggle in the read-only preview", () => {
    const html = render("/dise", { writable: false, sourceLabel: "开发预览 · 只读" });
    expect(html.match(/<button type="button" class="btn btn--small[^"]*" disabled=""/g)).toHaveLength(6);
    expect(html).toContain("只读预览");
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

  it("shows the audit checklist read-only on its tab", () => {
    const html = render("/rulings?tab=audit");
    expect(html.match(/<li class="audit-row( is-checked)?">/g)?.length).toBe(6);
    expect(text(html)).toContain("I-07");
    expect(html).toContain("已认可");
    expect(html).not.toMatch(/<input/);
  });
});

describe("知识", () => {
  it("groups pages into 判例, wiki folders and 体系", () => {
    const html = render("/knowledge");
    const groups = [...html.matchAll(/class="catalog__title">([^<]+)</g)].map((m) => m[1]);
    expect(groups).toEqual(["判例", "概念", "人物与公司", "书", "体系"]);
    expect(html).toContain("写下来的过程，就是把模糊的想法变清楚的过程。");
  });

  it("searches titles and full text from the URL query", () => {
    const html = render(`/knowledge?q=${encodeURIComponent("重复确认")}`);
    expect(text(html)).toContain("找到 1 页");
    expect(html).toContain('<mark class="hl">重复确认</mark>');
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
