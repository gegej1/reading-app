import { describe, expect, it } from "vitest";
import {
  createDesktop,
  describeCspViolation,
  describeError,
  describeRejection,
  forwardErrorsToLog,
  kbLoadSummary,
  logText,
  openExternalLink,
} from "./desktop";

function fakeInvoke(fail: Record<string, string> = {}) {
  const calls: [string, Record<string, unknown> | undefined][] = [];
  const invoke = async (command: string, args?: Record<string, unknown>) => {
    calls.push([command, args]);
    if (fail[command]) throw new Error(fail[command]);
    return null;
  };
  return { invoke, calls };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("desktop log", () => {
  it("writes info, warn and error lines through tauri-plugin-log", async () => {
    const { invoke, calls } = fakeInvoke();
    const desktop = createDesktop({ active: true, invoke });
    desktop.log("info", "kb loaded: files=3 brokenLinks=0 store=tauri");
    desktop.log("error", "boom");
    await flush();
    expect(calls).toEqual([
      ["plugin:log|log", { level: 3, message: "kb loaded: files=3 brokenLinks=0 store=tauri", location: "app" }],
      ["plugin:log|log", { level: 5, message: "boom", location: "app" }],
    ]);
  });

  it("does nothing on the web", async () => {
    const { invoke, calls } = fakeInvoke();
    const web = createDesktop({ active: false, invoke });
    web.log("error", "boom");
    await expect(web.openUrl("https://example.com")).rejects.toThrow();
    expect(forwardErrorsToLog(web, new EventTarget())).toBeTypeOf("function");
    await flush();
    expect(calls).toEqual([]);
  });

  it("swallows failures of the log command", async () => {
    const { invoke } = fakeInvoke({ "plugin:log|log": "not allowed" });
    const desktop = createDesktop({ active: true, invoke });
    expect(() => desktop.log("info", "x")).not.toThrow();
    await flush();
  });

  it("summarizes a load without any file names or contents", () => {
    const model = { docs: new Array(52).fill({}), links: { broken: [{}, {}] } };
    expect(kbLoadSummary("kb loaded", model, { kind: "tauri" })).toBe(
      "kb loaded: files=52 brokenLinks=2 store=tauri",
    );
    expect(kbLoadSummary("kb reloaded", model, { kind: "memory" })).toBe(
      "kb reloaded: files=52 brokenLinks=2 store=memory",
    );
  });

  it("keeps lines short and on one line", () => {
    expect(logText("第一行\n  第二行")).toBe("第一行 ⏎ 第二行");
    const long = logText("字".repeat(600));
    expect(long).toHaveLength(501);
    expect(long.endsWith("…")).toBe(true);
  });
});

describe("error forwarding", () => {
  it("logs uncaught errors, rejections and CSP violations, and can stop", async () => {
    const { invoke, calls } = fakeInvoke();
    const desktop = createDesktop({ active: true, invoke });
    const target = new EventTarget();
    const stop = forwardErrorsToLog(desktop, target);

    const error = Object.assign(new Event("error"), {
      message: "Uncaught TypeError: x is undefined",
      filename: "tauri://localhost/assets/index.js",
      lineno: 1,
      colno: 42,
      error: new TypeError("x is undefined"),
    });
    target.dispatchEvent(error);
    target.dispatchEvent(Object.assign(new Event("unhandledrejection"), { reason: new Error("没有找到") }));
    target.dispatchEvent(
      Object.assign(new Event("securitypolicyviolation"), {
        effectiveDirective: "script-src",
        blockedURI: "https://cdn.example.com/x.js",
        sourceFile: "",
        lineNumber: 0,
      }),
    );
    stop();
    target.dispatchEvent(Object.assign(new Event("unhandledrejection"), { reason: "after stop" }));
    await flush();

    expect(calls.map(([, args]) => [args?.level, args?.message])).toEqual([
      [5, "uncaught error: TypeError: x is undefined at tauri://localhost/assets/index.js:1:42"],
      [5, "unhandled rejection: Error: 没有找到"],
      [5, "CSP blocked script-src: https://cdn.example.com/x.js"],
    ]);
  });

  it("describes errors without an Error object", () => {
    expect(describeError({ message: "Script error.", filename: "", lineno: 0, colno: 0, error: null })).toBe(
      "uncaught error: Script error.",
    );
    expect(describeRejection("字符串原因")).toBe("unhandled rejection: 字符串原因");
    expect(describeRejection({ code: 7 })).toBe('unhandled rejection: {"code":7}');
    expect(
      describeCspViolation({ effectiveDirective: "style-src-elem", blockedURI: "", sourceFile: "a.js", lineNumber: 3 }),
    ).toBe("CSP blocked style-src-elem: inline at a.js:3");
  });
});

describe("external links", () => {
  it("open in the default browser on the desktop", async () => {
    const { invoke, calls } = fakeInvoke();
    const desktop = createDesktop({ active: true, invoke });
    let prevented = false;
    openExternalLink({ preventDefault: () => (prevented = true) }, "https://example.com/a?b=1", desktop);
    await flush();
    expect(prevented).toBe(true);
    expect(calls).toEqual([["plugin:opener|open_url", { url: "https://example.com/a?b=1" }]]);
  });

  it("are left to the browser on the web", () => {
    const { invoke, calls } = fakeInvoke();
    let prevented = false;
    openExternalLink({ preventDefault: () => (prevented = true) }, "https://example.com", createDesktop({ active: false, invoke }));
    expect(prevented).toBe(false);
    expect(calls).toEqual([]);
  });

  it("only opens http(s) URLs and logs failures", async () => {
    const { invoke, calls } = fakeInvoke({ "plugin:opener|open_url": "Not allowed to open url" });
    const desktop = createDesktop({ active: true, invoke });
    await expect(desktop.openUrl("file:///etc/passwd")).rejects.toThrow("http(s)");
    await expect(desktop.openUrl("https://example.com")).rejects.toThrow("Not allowed");
    await flush();
    expect(calls.map(([command]) => command)).toEqual(["plugin:opener|open_url", "plugin:log|log"]);
    expect(calls[1]?.[1]).toMatchObject({ level: 4, message: "open url failed: Not allowed to open url" });
  });
});
