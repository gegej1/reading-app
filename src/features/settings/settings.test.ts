import { describe, expect, it } from "vitest";
import { checkAiForm, formFromSettings, remoteText } from "./settings";
import { recordShortcut, type KeyPress } from "./shortcutKeys";

const press = (code: string, mods: Partial<KeyPress> = {}, key = "x"): KeyPress => ({
  key,
  code,
  metaKey: false,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  ...mods,
});

describe("the AI form", () => {
  it("turns what was typed into settings, an empty model meaning the engine's default", () => {
    expect(checkAiForm({ engine: "codex", model: " gpt-5 ", budget: "1.5", timeout: "20" })).toEqual({
      value: { engine: "codex", model: "gpt-5", budgetUsd: 1.5, runTimeoutMinutes: 20 },
      errors: null,
    });
    expect(checkAiForm({ engine: "claude", model: "", budget: "10", timeout: "30" }).value?.model).toBeNull();
  });

  it("names every field that makes no sense", () => {
    const result = checkAiForm({ engine: "claude", model: "a b", budget: "0", timeout: "2000" });
    expect(result.value).toBeNull();
    expect(result.errors).toEqual({
      model: "模型名里不能有空格",
      budget: "要是大于 0、不超过 1000 的美元数",
      timeout: "要是 1 到 1440 之间的分钟数",
    });
    expect(checkAiForm({ engine: "claude", model: "", budget: "", timeout: "abc" }).errors).toMatchObject({
      budget: expect.any(String),
      timeout: expect.any(String),
    });
    expect(checkAiForm({ engine: "claude", model: "m".repeat(121), budget: "1", timeout: "1" }).errors?.model).toContain("太长");
  });

  it("starts from the settings in effect", () => {
    expect(
      formFromSettings({
        engine: "claude",
        model: null,
        budgetUsd: 10,
        runTimeoutMinutes: 30,
        claudePath: null,
        codexPath: null,
        claudeProgram: null,
        codexProgram: null,
        problems: [],
      }),
    ).toEqual({ engine: "claude", model: "", budget: "10", timeout: "30" });
  });

  it("says where syncing goes without showing a URL", () => {
    expect(remoteText({ name: "origin", host: "github.com", localPath: null, tracked: true })).toBe("同步到 github.com（origin）");
    expect(remoteText({ name: "origin", host: null, localPath: "/tmp/remote.git", tracked: true })).toBe(
      "同步到 这台 Mac 上的文件夹 /tmp/remote.git（origin）",
    );
    expect(remoteText({ name: "backup", host: "example.org", localPath: null, tracked: false })).toContain("还没连上");
    expect(remoteText(null)).toContain("只保存在这台 Mac 上");
  });
});

describe("the shortcut recorder", () => {
  it("turns a key press into the setting and the Mac label", () => {
    expect(recordShortcut(press("KeyK", { ctrlKey: true, altKey: true }))).toEqual({
      setting: "Ctrl+Alt+K",
      label: "⌃⌥K",
      problem: null,
    });
    expect(recordShortcut(press("Space", { metaKey: true, shiftKey: true }))).toEqual({
      setting: "Shift+Cmd+Space",
      label: "⇧⌘Space",
      problem: null,
    });
    expect(recordShortcut(press("Slash", { ctrlKey: true, altKey: true }))?.label).toBe("⌃⌥/");
    expect(recordShortcut(press("Digit1", { ctrlKey: true, altKey: true }))?.setting).toBe("Ctrl+Alt+1");
    expect(recordShortcut(press("F5", { ctrlKey: true }))).toMatchObject({ setting: "Ctrl+F5", problem: null });
  });

  it("waits while only modifiers are held or an input method composes", () => {
    expect(recordShortcut(press("ControlLeft", { ctrlKey: true }, "Control"))).toBeNull();
    expect(recordShortcut(press("KeyK", { ctrlKey: true, altKey: true, isComposing: true }))).toBeNull();
  });

  it("refuses shortcuts that collide with macOS or with typing, in the backend's words", () => {
    const problem = (code: string, mods: Partial<KeyPress>) => recordShortcut(press(code, mods))?.problem ?? "";
    expect(problem("KeyK", { shiftKey: true })).toContain("⌃、⌥ 或 ⌘");
    expect(problem("Space", { metaKey: true })).toContain("Spotlight");
    expect(problem("Space", { ctrlKey: true })).toContain("输入法");
    expect(problem("Tab", { metaKey: true })).toContain("切换 App");
    expect(problem("KeyQ", { ctrlKey: true, metaKey: true })).toContain("锁定屏幕");
    expect(problem("Digit4", { metaKey: true, shiftKey: true })).toContain("截屏");
    expect(problem("KeyK", { altKey: true })).toContain("特殊字符");
    expect(problem("KeyK", { altKey: true, shiftKey: true })).toContain("特殊字符");
    expect(problem("KeyK", { metaKey: true })).toContain("只带一个修饰键");
    expect(problem("ArrowLeft", { ctrlKey: true })).toContain("只带一个修饰键");
    expect(recordShortcut(press("IntlBackslash", { ctrlKey: true, altKey: true }))?.problem).toContain("不能用");
    // Allowed: two modifiers, or ⌘⇧ with a key, or a function key.
    for (const [code, mods] of [
      ["KeyN", { ctrlKey: true, altKey: true }],
      ["KeyK", { metaKey: true, shiftKey: true }],
      ["KeyK", { ctrlKey: true, shiftKey: true }],
      ["F2", { altKey: true }],
    ] as const) {
      expect(problem(code, mods), code).toBe("");
    }
  });
});
