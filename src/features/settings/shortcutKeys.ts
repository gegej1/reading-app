// The key recorder of the settings screen: a key press becomes a shortcut
// setting (`Ctrl+Alt+K`, the spelling config.json and the backend read),
// with the label Mac menus use (⌃⌥K). The checks mirror
// src-tauri/src/shortcut.rs, which checks again before registering.

export interface KeyPress {
  key: string;
  code: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  isComposing?: boolean;
}

export interface RecordedShortcut {
  /** For config.json: `Ctrl+Alt+K`. */
  setting: string;
  /** As Mac menus show it: `⌃⌥K`. */
  label: string;
  /** Why it cannot be used, or null. */
  problem: string | null;
}

const MODIFIER_KEYS = new Set(["Control", "Alt", "Shift", "Meta", "OS", "CapsLock", "Fn", "Hyper", "Super"]);

const PUNCTUATION: Record<string, string> = {
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
};

const NAMED: Record<string, string> = {
  Space: "Space",
  Enter: "↩",
  Tab: "⇥",
  Escape: "⎋",
  Backspace: "⌫",
  Delete: "⌦",
  ArrowUp: "↑",
  ArrowDown: "↓",
  ArrowLeft: "←",
  ArrowRight: "→",
  Home: "↖",
  End: "↘",
  PageUp: "⇞",
  PageDown: "⇟",
};

/** The key part of a setting and its label, for a `KeyboardEvent.code`; null for keys that cannot be used. */
function keyOf(code: string): { setting: string; label: string; types: boolean } | null {
  const letter = /^Key([A-Z])$/.exec(code)?.[1];
  if (letter) return { setting: letter, label: letter, types: true };
  const digit = /^Digit([0-9])$/.exec(code)?.[1];
  if (digit) return { setting: digit, label: digit, types: true };
  if (/^F([1-9]|1[0-2])$/.test(code)) return { setting: code, label: code, types: false };
  const mark = PUNCTUATION[code];
  if (mark) return { setting: code, label: mark, types: true };
  const named = NAMED[code];
  if (named) return { setting: code, label: named, types: true };
  return null;
}

/** True while the press holds only modifier keys (the shortcut is not complete yet). */
export function onlyModifiers(press: Pick<KeyPress, "key">): boolean {
  return MODIFIER_KEYS.has(press.key);
}

/** The modifiers held, as Mac menus show them (⌃⌥⇧⌘). */
export function modifierLabel(press: Pick<KeyPress, "ctrlKey" | "altKey" | "shiftKey" | "metaKey">): string {
  return `${press.ctrlKey ? "⌃" : ""}${press.altKey ? "⌥" : ""}${press.shiftKey ? "⇧" : ""}${press.metaKey ? "⌘" : ""}`;
}

/**
 * The shortcut a key press asks for, with what is wrong with it; null while
 * only modifiers are held or during an input-method composition.
 */
export function recordShortcut(press: KeyPress): RecordedShortcut | null {
  if (press.isComposing || onlyModifiers(press)) return null;
  const key = keyOf(press.code);
  const label = `${modifierLabel(press)}${key?.label ?? press.key}`;
  if (!key) return { setting: "", label, problem: "这个键不能用在快捷键里，换一个吧" };
  const parts = [press.ctrlKey && "Ctrl", press.altKey && "Alt", press.shiftKey && "Shift", press.metaKey && "Cmd"].filter(
    (part): part is string => typeof part === "string",
  );
  const setting = [...parts, key.setting].join("+");
  return { setting, label, problem: shortcutProblem(press, key.setting, key.types, label) };
}

function shortcutProblem(
  press: Pick<KeyPress, "ctrlKey" | "altKey" | "shiftKey" | "metaKey">,
  key: string,
  types: boolean,
  label: string,
): string | null {
  const { ctrlKey: ctrl, altKey: alt, shiftKey: shift, metaKey: cmd } = press;
  if (!ctrl && !alt && !cmd) return "至少要带 ⌃、⌥ 或 ⌘ 中的一个，不然会挡住平常打字";
  const only = (want: { ctrl?: boolean; alt?: boolean; cmd?: boolean }) =>
    ctrl === Boolean(want.ctrl) && alt === Boolean(want.alt) && cmd === Boolean(want.cmd);
  const system: [boolean, string][] = [
    [only({ cmd: true }) && !shift && key === "Space", "Spotlight 搜索"],
    [only({ ctrl: true }) && !shift && key === "Space", "切换输入法"],
    [only({ cmd: true, alt: true }) && key === "Space", "Finder 搜索"],
    [only({ ctrl: true, cmd: true }) && key === "Space", "表情与符号"],
    [only({ cmd: true }) && key === "Tab", "切换 App"],
    [only({ cmd: true }) && key === "Backquote", "切换窗口"],
    [only({ ctrl: true, cmd: true }) && key === "Q", "锁定屏幕"],
    [only({ cmd: true }) && shift && key === "Q", "退出登录"],
    [only({ cmd: true, alt: true }) && key === "Escape", "强制退出"],
    [only({ cmd: true }) && shift && ["3", "4", "5"].includes(key), "截屏"],
    [only({ cmd: true, alt: true }) && key === "D", "显示或隐藏程序坞"],
  ];
  const hit = system.find(([applies]) => applies);
  if (hit) return `${label} 是 macOS 的「${hit[1]}」，换一个吧`;
  if (types) {
    if (only({ alt: true })) return `${label} 是用来打特殊字符的，再加一个 ⌃ 或 ⌘，比如 ⌃⌥N`;
    if ((only({ cmd: true }) || only({ ctrl: true })) && !shift) {
      return `${label} 只带一个修饰键，各个 App 自己在用这样的快捷键（或者用它编辑文字）。再加一个修饰键，比如 ⌃⌥N`;
    }
  }
  return null;
}
