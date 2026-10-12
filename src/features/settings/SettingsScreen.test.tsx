// @vitest-environment jsdom
// The settings screen against a fake backend: checking the form before
// saving, the folder check (an invalid folder cannot be chosen), the
// shortcut recorder (conflicts with macOS, a shortcut another app holds),
// launch at login, and nothing at all on the web.

import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter } from "react-router";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { createDesktop } from "../../app/desktop";
import { KbContext, type KbContextValue } from "../../app/KbContext";
import { makeLinkResolver } from "../../app/links";
import { buildKbModel } from "../../domain";
import { fixtureKb } from "../../domain/__fixtures__";
import { createMemoryStore } from "../../store/memoryStore";
import type { SettingsInfo } from "./settings";
import SettingsScreen from "./SettingsScreen";

beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

let root: Root | null = null;
let container: HTMLElement | null = null;

afterEach(() => {
  act(() => root?.unmount());
  container?.remove();
  root = null;
  container = null;
});

async function settle(ms = 0) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

async function waitFor(check: () => boolean, what: string) {
  for (let i = 0; i < 250; i += 1) {
    if (check()) return;
    await settle(10);
  }
  throw new Error(`timed out waiting for ${what}: ${container?.textContent?.slice(0, 300)}`);
}

const INFO: SettingsInfo = {
  version: "0.1.0",
  run: {
    engine: "claude",
    model: null,
    budgetUsd: 10,
    runTimeoutMinutes: 30,
    claudePath: null,
    codexPath: null,
    claudeProgram: "/opt/homebrew/bin/claude",
    codexProgram: null,
    problems: [],
  },
  kb: {
    path: "/Users/someone/kb",
    source: "config",
    check: {
      path: "/Users/someone/kb",
      ok: true,
      problem: null,
      hasRules: true,
      remote: { name: "origin", host: "github.com", localPath: null, tracked: true },
    },
  },
  kbProblem: null,
  captureShortcut: { setting: "Ctrl+Alt+N", label: "⌃⌥N", registered: true, error: null },
  autostart: false,
  configProblem: null,
  bridge: null,
};

function backend(handlers: Record<string, (args: Record<string, unknown> | undefined) => unknown>, active = true) {
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const bridge = createDesktop({
    active,
    invoke: async (command, args) => {
      if (command === "plugin:log|log") return undefined;
      calls.push({ command, args });
      const handler = handlers[command];
      if (!handler) throw new Error(`unexpected command ${command}`);
      return handler(args);
    },
    listen: async () => () => undefined,
  });
  return { bridge, called: (command: string) => calls.filter((call) => call.command === command) };
}

function mount(bridge: ReturnType<typeof backend>["bridge"], storeKind: "tauri" | "memory" = "tauri") {
  const model = buildKbModel(Object.entries(fixtureKb).map(([path, content]) => ({ path, content })));
  const kb: KbContextValue = {
    store: { ...createMemoryStore(fixtureKb), kind: storeKind },
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
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
  act(() =>
    root?.render(
      <KbContext.Provider value={kb}>
        <MemoryRouter>
          <SettingsScreen bridge={bridge} />
        </MemoryRouter>
      </KbContext.Provider>,
    ),
  );
}

const button = (label: string) =>
  [...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent?.trim() === label);
const click = (element: Element | undefined | null) => act(() => (element as HTMLElement | null)?.click());

function type(input: HTMLInputElement, value: string) {
  const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  act(() => {
    setValue?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const inputAfter = (label: string) =>
  [...(container?.querySelectorAll(".field") ?? [])]
    .find((field) => field.querySelector(".field__label")?.textContent === label)
    ?.querySelector("input") as HTMLInputElement;

function press(init: KeyboardEventInit) {
  act(() => {
    window.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init }));
  });
}

describe("the AI settings", () => {
  it("checks every field before saving and saves what was typed", async () => {
    const fake = backend({
      settings_get: () => INFO,
      settings_save_ai: (args) => ({ ...INFO, run: { ...INFO.run, ...(args?.settings as object) } }),
    });
    mount(fake.bridge);
    await waitFor(() => button("保存") !== undefined, "the form");
    expect(button("保存")?.disabled).toBe(true);
    expect(container?.textContent).toContain("找到了：/opt/homebrew/bin/claude");
    expect(container?.textContent).toContain("没在这台 Mac 上找到 codex");

    type(inputAfter("每次运行的花费上限"), "0");
    type(inputAfter("运行时限"), "abc");
    type(inputAfter("模型"), "claude sonnet");
    click(button("保存"));
    const errors = [...(container?.querySelectorAll(".field__error") ?? [])].map((error) => error.textContent);
    expect(errors).toEqual(["模型名里不能有空格", "要是大于 0、不超过 1000 的美元数", "要是 1 到 1440 之间的分钟数"]);
    expect(fake.called("settings_save_ai")).toEqual([]);

    type(inputAfter("模型"), " sonnet ");
    type(inputAfter("每次运行的花费上限"), "1.5");
    type(inputAfter("运行时限"), "20");
    click(button("保存"));
    await waitFor(() => fake.called("settings_save_ai").length === 1, "the save");
    expect(fake.called("settings_save_ai")[0]?.args).toEqual({
      settings: { engine: "claude", model: "sonnet", budgetUsd: 1.5, runTimeoutMinutes: 20 },
    });
  });

  it("will not save while config.json cannot be read", async () => {
    const fake = backend({ settings_get: () => ({ ...INFO, configProblem: "config.json 不是有效的 JSON。为了不丢掉里面的内容，这次没有保存" }) });
    mount(fake.bridge);
    await waitFor(() => container?.querySelector(".settings-banner") != null, "the banner");
    type(inputAfter("运行时限"), "20");
    expect(button("保存")?.disabled).toBe(true);
    expect(button("更换…")?.disabled).toBe(true);
  });
});

describe("the knowledge-base folder", () => {
  it("shows where syncing goes, and an invalid folder cannot be chosen", async () => {
    const fake = backend({
      settings_get: () => INFO,
      settings_pick_kb: () => ({
        path: "/Users/someone/Documents",
        check: {
          path: "/Users/someone/Documents",
          ok: false,
          problem: "这个文件夹不是 git 仓库。知识库要是一个 git 仓库的根目录（里面有 .git）",
          hasRules: false,
          remote: null,
        },
      }),
    });
    mount(fake.bridge);
    await waitFor(() => button("更换…") !== undefined, "the section");
    expect(container?.textContent).toContain("/Users/someone/kb");
    expect(container?.textContent).toContain("同步到 github.com（origin）");
    click(button("更换…"));
    await waitFor(() => container?.querySelector(".kb-candidate") != null, "the candidate");
    const candidate = container?.querySelector(".kb-candidate")?.textContent ?? "";
    expect(candidate).toContain("/Users/someone/Documents");
    expect(candidate).toContain("这个文件夹不是 git 仓库");
    expect(button("用这个文件夹")?.disabled).toBe(true);
    click(button("用这个文件夹"));
    expect(fake.called("settings_set_kb")).toEqual([]);
    click(button("算了"));
    expect(container?.querySelector(".kb-candidate")).toBeNull();
  });

  it("warns about a folder without AGENTS.md and a remote the branch does not follow", async () => {
    const fake = backend({
      settings_get: () => INFO,
      settings_pick_kb: () => ({
        path: "/tmp/kb",
        check: { path: "/tmp/kb", ok: true, problem: null, hasRules: false, remote: { name: "backup", host: null, localPath: "/tmp/remote.git", tracked: false } },
      }),
    });
    mount(fake.bridge);
    await waitFor(() => button("更换…") !== undefined, "the section");
    click(button("更换…"));
    await waitFor(() => container?.querySelector(".kb-candidate") != null, "the candidate");
    const candidate = container?.querySelector(".kb-candidate")?.textContent ?? "";
    expect(candidate).toContain("没有 AGENTS.md");
    expect(candidate).toContain("这台 Mac 上的文件夹 /tmp/remote.git");
    expect(candidate).toContain("还没连上它");
    expect(button("用这个文件夹")?.disabled).toBe(false);
  });
});

describe("the capture shortcut", () => {
  it("records a key press, refuses one macOS uses, and reports one another app holds", async () => {
    let attempt = 0;
    const fake = backend({
      settings_get: () => INFO,
      settings_set_shortcut: (args) => {
        attempt += 1;
        if (attempt === 1) throw new Error("⌃⌥K 没能设置，可能已经被别的 App 占用了。原来的快捷键照常可用，换一个试试");
        return { setting: args?.setting, label: "⌃⌥J", registered: true, error: null };
      },
    });
    mount(fake.bridge);
    await waitFor(() => button("录新的快捷键") !== undefined, "the recorder");
    expect(container?.querySelector(".keycap")?.textContent).toBe("⌃⌥N");

    click(button("录新的快捷键"));
    expect(container?.textContent).toContain("按下新的快捷键…");
    press({ key: "Meta", code: "MetaLeft", metaKey: true });
    expect(container?.textContent).toContain("⌘…");
    press({ key: " ", code: "Space", metaKey: true });
    expect(container?.textContent).toContain("⌘Space 是 macOS 的「Spotlight 搜索」");
    expect(fake.called("settings_set_shortcut")).toEqual([]);

    click(button("录新的快捷键"));
    press({ key: "k", code: "KeyK", ctrlKey: true, altKey: true });
    await waitFor(() => fake.called("settings_set_shortcut").length === 1, "the first try");
    expect(fake.called("settings_set_shortcut")[0]?.args).toEqual({ setting: "Ctrl+Alt+K" });
    await waitFor(() => (container?.textContent ?? "").includes("已经被别的 App 占用了"), "the conflict");
    expect(container?.querySelector(".keycap")?.textContent).toBe("⌃⌥N");

    click(button("录新的快捷键"));
    press({ key: "j", code: "KeyJ", ctrlKey: true, altKey: true });
    await waitFor(() => (container?.textContent ?? "").includes("已换成 ⌃⌥J"), "the new shortcut");
    expect(container?.querySelector(".keycap")?.textContent).toBe("⌃⌥J");
  });

  it("Esc stops recording without changing anything", async () => {
    const fake = backend({ settings_get: () => INFO });
    mount(fake.bridge);
    await waitFor(() => button("录新的快捷键") !== undefined, "the recorder");
    click(button("录新的快捷键"));
    press({ key: "Escape", code: "Escape" });
    expect(container?.textContent).not.toContain("按下新的快捷键");
    expect(fake.called("settings_set_shortcut")).toEqual([]);
  });
});

describe("launch at login", () => {
  it("is off by default and turns on only when asked", async () => {
    const fake = backend({ settings_get: () => INFO, settings_set_autostart: (args) => args?.enabled === true });
    mount(fake.bridge);
    await waitFor(() => container?.querySelector('input[role="switch"]') != null, "the switch");
    const toggle = container?.querySelector('input[role="switch"]') as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    expect(container?.textContent).toContain("默认关闭");
    expect(fake.called("settings_set_autostart")).toEqual([]);
    click(toggle);
    await waitFor(() => fake.called("settings_set_autostart").length === 1, "the toggle");
    expect(fake.called("settings_set_autostart")[0]?.args).toEqual({ enabled: true });
    await waitFor(() => (container?.querySelector('input[role="switch"]') as HTMLInputElement).checked, "on");
  });
});

describe("the iPhone bridge", () => {
  const BRIDGE = {
    enabled: true,
    path: "/Users/someone/Library/Mobile Documents/iCloud~is~workflow~my~workflows/Documents/底色",
    source: "default" as const,
    ready: true,
    problem: null,
    lastImport: "2026-10-12 09:30 收进 1 条记一笔",
    shortcuts: "3 个快捷指令已签好，放在「快捷指令」文件夹里",
    waiting: 0,
    importProblem: null,
  };

  it("warns when nothing can be imported", async () => {
    const problem = "导入记录写不进去（Permission denied），所以暂时不收手机上的文件，免得同一条收两次。";
    const info = { ...INFO, bridge: { ...BRIDGE, importProblem: problem } };
    mount(backend({ settings_get: () => info }).bridge);
    await waitFor(() => container?.textContent?.includes("和 iPhone 互通") === true, "the section");
    const alert = [...(container?.querySelectorAll('[role="alert"]') ?? [])].find((el) => el.textContent === problem);
    expect(alert).toBeDefined();
  });

  it("shows where the folder is and turns off when asked", async () => {
    const info = { ...INFO, bridge: BRIDGE };
    const fake = backend({
      settings_get: () => info,
      settings_set_bridge: (args) => ({ ...info, bridge: { ...BRIDGE, enabled: args?.enabled === true } }),
      settings_open_folder: () => null,
    });
    mount(fake.bridge);
    await waitFor(() => container?.textContent?.includes("和 iPhone 互通") === true, "the section");
    const text = container?.textContent ?? "";
    expect(text).toContain("iCloud 云盘 › 快捷指令 › 底色");
    expect(text).toContain("收进 1 条记一笔");
    expect(text).toContain("3 个快捷指令已签好");
    expect(text).not.toContain("Mobile Documents");
    expect(text).not.toContain("注意");
    const opener = [...(container?.querySelectorAll("button") ?? [])].find((b) => b.textContent === "在访达中打开");
    click(opener as HTMLElement);
    await waitFor(() => fake.called("settings_open_folder").length === 1, "the folder");
    expect(fake.called("settings_open_folder")[0]?.args).toEqual({ which: "bridge" });

    const toggle = [...(container?.querySelectorAll<HTMLInputElement>('input[role="switch"]') ?? [])].find((input) =>
      input.closest("label")?.textContent?.includes("iPhone"),
    );
    expect(toggle?.checked).toBe(true);
    click(toggle as HTMLElement);
    await waitFor(() => fake.called("settings_set_bridge").length === 1, "the switch");
    expect(fake.called("settings_set_bridge")[0]?.args).toEqual({ enabled: false });
    await waitFor(() => !(container?.textContent ?? "").includes("正在看着"), "off");
  });
});

describe("on the web", () => {
  it("there are no settings", async () => {
    const fake = backend({}, false);
    mount(fake.bridge, "memory");
    expect(container?.textContent).toContain("设置只在 Mac 上的底色里");
  });
});
