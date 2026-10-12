// @vitest-environment jsdom
// Answering a pending ruling and auditing an ingest report in a DOM: the
// files that are created (path, frontmatter, verbatim words, commit
// message), ticking an item, and what an AI run blocks.

import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { AppRoutes } from "../../App";
import { KbProvider } from "../../app/KbProvider";
import { pageRoute } from "../../app/links";
import { NO_RUNS, RunContext, type RunContextValue } from "../../app/RunContext";
import { parseFrontmatter } from "../../domain";
import { fixtureKb } from "../../domain/__fixtures__";
import { createMemoryStore, type MemoryStore } from "../../store/memoryStore";

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  Element.prototype.scrollTo ??= () => undefined;
});

let root: Root | null = null;
let container: HTMLElement | null = null;

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  root = null;
  container = null;
  localStorage.clear();
});

async function settle(ms = 0) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

async function waitFor(check: () => boolean, what: string) {
  for (let i = 0; i < 500; i += 1) {
    if (check()) return;
    await settle(10);
  }
  throw new Error(`timed out waiting for ${what}`);
}

function mount(store: MemoryStore, route: string, runs: Partial<RunContextValue> = {}) {
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  const tree: ReactNode = (
    <KbProvider store={store}>
      <RunContext.Provider value={{ ...NO_RUNS, ...runs }}>
        <MemoryRouter initialEntries={[route]}>
          <AppRoutes />
        </MemoryRouter>
      </RunContext.Provider>
    </KbProvider>
  );
  act(() => root?.render(tree));
  return container;
}

function type(textarea: HTMLTextAreaElement, value: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
  act(() => {
    setValue?.call(textarea, value);
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const button = (scope: ParentNode, label: string) =>
  [...scope.querySelectorAll("button")].find((b) => b.textContent?.trim() === label) ?? null;

const Q001 = pageRoute("self/待裁决/Q-001-周会要不要取消.md");
const REPORT = pageRoute("reports/2026-09-08-入库报告-2.md");

describe("answering a pending ruling", () => {
  it("creates inbox/裁决/Q-001-<time>-web.md with 对象, 时间, 来源 and the words verbatim", async () => {
    const store = createMemoryStore(fixtureKb, { now: () => 0 });
    const page = mount(store, Q001);
    await waitFor(() => page.querySelector('[aria-label="回答 Q-001"] textarea') !== null, "the answer box");
    const box = page.querySelector<HTMLTextAreaElement>('[aria-label="回答 Q-001"] textarea');
    if (!box) throw new Error("no answer box");
    // Drafts are kept per question. (A text box already reports line breaks
    // as LF; CRLF from elsewhere is covered by the domain tests.)
    type(box, "  先保留，\n但只开半小时。\n\n");
    expect(localStorage.getItem("dise.answer.draft.Q-001")).toBe("  先保留，\n但只开半小时。\n\n");

    act(() => {
      box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true, cancelable: true }));
    });
    await waitFor(() => store.commits.length === 1, "the answer to be created");

    const created = store.commits[0];
    expect(created?.message).toBe("收件箱：回答 Q-001（web）");
    expect(created?.path).toMatch(/^inbox\/裁决\/Q-001-\d{4}-\d\d-\d\dT\d\d-\d\d-\d\d-web\.md$/);
    const content = await store.read(created?.path ?? "");
    const { data, body } = parseFrontmatter(content);
    expect(Object.keys(data)).toEqual(["对象", "时间", "来源"]);
    expect(data["对象"]).toBe("Q-001");
    expect(data["来源"]).toBe("web");
    expect(String(data["时间"])).toMatch(/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d[+-]\d\d:\d\d$/);
    expect(body).toBe("\n  先保留，\n但只开半小时。\n");
    await waitFor(() => localStorage.getItem("dise.answer.draft.Q-001") === null, "the draft to be cleared");

    // Both answers now wait for 落裁决; another one can follow.
    await waitFor(() => page.querySelectorAll(".answer__item").length === 2, "the queued answers");
    expect(page.textContent).toContain("已回答，待落裁决");
    expect(page.querySelector('[aria-label="回答 Q-001"] textarea')).not.toBeNull();
  });

  it("waits while a 落裁决 works through inbox/裁决/, keeping the draft", async () => {
    const store = createMemoryStore(fixtureKb);
    const page = mount(store, Q001, { available: true, answerBlocked: "正在落裁决，结束后再回答" });
    await waitFor(() => page.querySelector('[aria-label="回答 Q-001"] textarea') !== null, "the answer box");
    const slip = page.querySelector('[aria-label="回答 Q-001"]');
    const box = slip?.querySelector("textarea");
    if (!slip || !box) throw new Error("no answer box");
    type(box, "等会儿再交");
    const submit = button(slip, "交上回答");
    expect(submit?.disabled).toBe(true);
    expect(slip.textContent).toContain("正在落裁决，结束后再回答");
    act(() => {
      box.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true, cancelable: true }));
    });
    await settle(20);
    expect(store.commits).toEqual([]);
    expect(localStorage.getItem("dise.answer.draft.Q-001")).toBe("等会儿再交");
  });

  it("still takes answers while an ingest runs", async () => {
    const store = createMemoryStore(fixtureKb);
    const page = mount(store, Q001, { available: true, editBlocked: "正在入库，结束后再改", answerBlocked: null });
    await waitFor(() => page.querySelector('[aria-label="回答 Q-001"] textarea') !== null, "the answer box");
    const slip = page.querySelector('[aria-label="回答 Q-001"]');
    const box = slip?.querySelector("textarea");
    if (!slip || !box) throw new Error("no answer box");
    type(box, "入库的时候也能回答");
    act(() => button(slip, "交上回答")?.click());
    await waitFor(() => store.commits.length === 1, "the answer");
    expect(store.commits[0]?.message).toBe("收件箱：回答 Q-001（web）");
  });
});

describe("auditing a report", () => {
  const row = (page: HTMLElement, id: string) =>
    [...page.querySelectorAll<HTMLElement>(".audit-row")].find((li) => li.textContent?.includes(id)) ?? null;

  it("ticks an item with the serialized, intent-based edit", async () => {
    const store = createMemoryStore(fixtureKb);
    const page = mount(store, REPORT);
    await waitFor(() => row(page, "I-07") !== null, "the checklist");
    const box = row(page, "I-07")?.querySelector<HTMLButtonElement>('[role="checkbox"]');
    expect(box?.getAttribute("aria-checked")).toBe("false");
    act(() => box?.click());
    await waitFor(() => store.commits.length === 1, "the commit");
    expect(store.commits[0]).toEqual({
      path: "reports/2026-09-08-入库报告-2.md",
      message: "抽查：认可 I-07（2026-09-08-入库报告-2）",
    });
    await waitFor(
      () => row(page, "I-07")?.querySelector('[role="checkbox"]')?.getAttribute("aria-checked") === "true",
      "the ticked item",
    );
    expect(await store.read("reports/2026-09-08-入库报告-2.md")).toContain("- [x] I-07 周五下午不安排会议");
  });

  it("不认可 opens a reason box and creates the rejection for 落裁决", async () => {
    const store = createMemoryStore(fixtureKb);
    const page = mount(store, REPORT);
    await waitFor(() => row(page, "I-07") !== null, "the checklist");
    act(() => button(row(page, "I-07") ?? page, "不认可")?.click());
    const reason = row(page, "I-07")?.querySelector<HTMLTextAreaElement>("textarea");
    if (!reason) throw new Error("no reason box");
    expect(document.activeElement).toBe(reason);
    type(reason, "周一也不该开长会。\n");
    act(() => button(row(page, "I-07") ?? page, "交上理由")?.click());
    await waitFor(() => store.commits.length === 1, "the rejection");

    const created = store.commits[0];
    expect(created?.message).toBe("收件箱：驳回抽查 I-07（2026-09-08-入库报告-2，web）");
    expect(created?.path).toMatch(
      /^inbox\/裁决\/抽查-2026-09-08-入库报告-2-I-07-\d{4}-\d\d-\d\dT\d\d-\d\d-\d\d-web\.md$/,
    );
    const { data, body } = parseFrontmatter(await store.read(created?.path ?? ""));
    expect(data["对象"]).toBe("抽查 2026-09-08-入库报告-2 I-07");
    expect(data["来源"]).toBe("web");
    expect(body).toBe("\n周一也不该开长会。\n");
    // The report itself is not touched.
    expect(await store.read("reports/2026-09-08-入库报告-2.md")).toBe(fixtureKb["reports/2026-09-08-入库报告-2.md"]);
    await waitFor(() => row(page, "I-07")?.textContent?.includes("已驳回，待落裁决") === true, "the rejected state");
    expect(row(page, "I-07")?.textContent).toContain("周一也不该开长会。");
  });

  it("while a run holds the knowledge base ticking waits with a hint; nothing is written", async () => {
    const store = createMemoryStore(fixtureKb);
    const page = mount(store, REPORT, { available: true, editBlocked: "正在入库，结束后再改" });
    await waitFor(() => row(page, "I-07") !== null, "the checklist");
    const box = row(page, "I-07")?.querySelector<HTMLButtonElement>('[role="checkbox"]');
    expect(box?.disabled).toBe(true);
    expect(box?.closest(".hint")?.getAttribute("data-hint")).toBe("正在入库，结束后再改");
    act(() => box?.click());
    await settle(20);
    expect(store.commits).toEqual([]);
  });
});
