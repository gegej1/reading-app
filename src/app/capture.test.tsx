// @vitest-environment jsdom
// The web and demo flow of 记一笔: the button on 今日 opens the slip in the
// page, ⌘↩ saves a capture into the store, and 今日 counts it right away.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import App from "../App";
import { fixtureKb } from "../domain/__fixtures__";
import { TodayScreen } from "../features/today/TodayScreen";
import { createMemoryStore } from "../store/memoryStore";
import { CaptureContext, type ShortcutStatus } from "./capture";
import { KbContext, type KbContextValue } from "./KbContext";
import { buildKbModel } from "../domain";
import { makeLinkResolver } from "./links";

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // jsdom has no layout, so no scrolling (the shell scrolls the desk to the top).
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
  // Generous: slow CI runners load the demo knowledge base slowly.
  for (let i = 0; i < 500; i += 1) {
    if (check()) return;
    await settle(10);
  }
  throw new Error(`timed out waiting for ${what}`);
}

const inboxCount = () =>
  container?.querySelector(".ledger__cell .ledger__number")?.textContent ?? "";

describe("记一笔 in the page", () => {
  it("opens from 今日, saves with ⌘↩ and the inbox count goes up", async () => {
    const store = createMemoryStore(fixtureKb, { now: () => new Date(2026, 9, 12, 14, 3, 22).getTime() });
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    act(() => root?.render(<App store={store} />));
    await waitFor(() => inboxCount() === "2", "the 今日 screen");

    const button = [...container.querySelectorAll("button")].find((b) => b.textContent?.includes("记一笔"));
    act(() => button?.click());
    const dialog = container.querySelector('[role="dialog"]');
    expect(dialog?.getAttribute("aria-label")).toBe("记一笔");
    const textarea = dialog?.querySelector("textarea");
    if (!textarea) throw new Error("no textarea");
    expect(document.activeElement).toBe(textarea);

    const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
    act(() => {
      setValue?.call(textarea, "散步时想到的一句话\n第二行");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    act(() => {
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", metaKey: true, bubbles: true, cancelable: true }));
    });
    await waitFor(() => inboxCount() === "3", "the new count");

    const created = store.commits.at(-1);
    expect(created?.message).toBe("收件箱：记一笔（web）");
    expect(created?.path).toMatch(/^inbox\/\d{4}-\d\d-\d\dT\d\d-\d\d-\d\d-web\.md$/);
    const content = await store.read(created?.path ?? "");
    expect(content).toMatch(/^---\n来源: web\n时间: .+\n---\n\n散步时想到的一句话\n第二行\n$/);

    // 已记下, then the dialog closes by itself.
    await waitFor(() => container?.querySelector('[role="dialog"]') === null, "the dialog to close");
  });

  it("closes on Esc and keeps the draft for next time", async () => {
    const store = createMemoryStore(fixtureKb);
    container = document.createElement("div");
    document.body.append(container);
    root = createRoot(container);
    act(() => root?.render(<App store={store} />));
    await waitFor(() => inboxCount() === "2", "the 今日 screen");

    const open = () =>
      act(() => [...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.includes("记一笔"))?.click());
    open();
    const textarea = container.querySelector("textarea");
    if (!textarea) throw new Error("no textarea");
    const setValue = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set;
    act(() => {
      setValue?.call(textarea, "还没写完");
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    act(() => {
      textarea.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    });
    expect(container.querySelector('[role="dialog"]')).toBeNull();
    expect(store.commits).toEqual([]);

    open();
    expect(container.querySelector("textarea")?.value).toBe("还没写完");
  });
});

describe("今日 and the desktop shortcut", () => {
  const model = buildKbModel(Object.entries(fixtureKb).map(([path, content]) => ({ path, content })));
  const kb: KbContextValue = {
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
    sourceLabel: "本机知识库",
    sourceDetail: null,
  };

  const render = (shortcut: ShortcutStatus | null) =>
    renderToStaticMarkup(
      <KbContext.Provider value={kb}>
        <CaptureContext.Provider value={{ open: () => undefined, shortcut }}>
          <MemoryRouter>
            <TodayScreen now={new Date(2026, 9, 12)} />
          </MemoryRouter>
        </CaptureContext.Provider>
      </KbContext.Provider>,
    );

  it("shows the shortcut on the button when it works", () => {
    const html = render({ setting: "Ctrl+Alt+N", label: "⌃⌥N", registered: true, error: null });
    expect(html).toContain('<kbd class="today__kbd">⌃⌥N</kbd>');
    expect(html).not.toContain("today__notice");
  });

  it("says so when the shortcut could not be registered", () => {
    const error = "快捷键 ⌃⌥N 没能注册，可能已经被别的 App 占用了。可以在 config.json 里用 captureShortcut 换一个。";
    const html = render({ setting: "Ctrl+Alt+N", label: "⌃⌥N", registered: false, error });
    expect(html).toContain('role="note"');
    expect(html).toContain(error);
    expect(html).toContain("菜单栏");
    expect(html).not.toContain("today__kbd");
  });
});
