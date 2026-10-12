import { describe, expect, it } from "vitest";
import { buildKbModel } from "../domain";
import { fixtureKb } from "../domain/__fixtures__";
import { createDesktop } from "./desktop";
import { createPhoneExporter, untilNextDay } from "./phoneExport";

const model = buildKbModel(Object.entries(fixtureKb).map(([path, content]) => ({ path, content })));

function fakeDesktop(active = true) {
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const listeners = new Map<string, (payload: unknown) => void>();
  const bridge = createDesktop({
    active,
    invoke: async (command, args) => {
      calls.push({ command, args });
      return { exported: true, written: 1, unchanged: 0, removed: 0 };
    },
    listen: async (event, handler) => {
      listeners.set(event, handler);
      return () => listeners.delete(event);
    },
  });
  const exports = () => calls.filter((call) => call.command === "bridge_export");
  return { bridge, exports, emit: (event: string) => listeners.get(event)?.(null) };
}

async function waitFor(check: () => boolean, what: string) {
  for (let i = 0; i < 200; i += 1) {
    if (check()) return;
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`timed out waiting for ${what}`);
}

describe("the iPhone export of the desktop app", () => {
  it("exports once after a burst of loads, and again when the folder is ready", async () => {
    const fake = fakeDesktop();
    const exporter = createPhoneExporter(fake.bridge, { delayMs: 20, now: () => new Date(2026, 9, 12, 9, 30) });
    exporter.loaded(model);
    exporter.loaded(model);
    exporter.loaded(model);
    await waitFor(() => fake.exports().length === 1, "the export");
    const files = fake.exports()[0]?.args?.files as { name: string; content: string }[];
    expect(files.map((file) => file.name)).toContain("今日.html");
    expect(files.map((file) => file.name)).toContain("待裁决/Q-001.txt");
    // Only the 底色 layer: no raw/ or wiki/ text.
    expect(fixtureKb["wiki/概念/小实验.md"]).toContain("围栏里的链接");
    expect(JSON.stringify(files)).not.toContain("围栏里的链接");

    await new Promise((resolve) => setTimeout(resolve, 60));
    expect(fake.exports()).toHaveLength(1);
    fake.emit("bridge-ready");
    await waitFor(() => fake.exports().length === 2, "the export after bridge-ready");
    exporter.stop();
  });

  it("does nothing on the web", async () => {
    const fake = fakeDesktop(false);
    const exporter = createPhoneExporter(fake.bridge, { delayMs: 1 });
    exporter.loaded(model);
    await new Promise((resolve) => setTimeout(resolve, 30));
    expect(fake.exports()).toHaveLength(0);
    exporter.stop();
  });

  it("wakes up a minute after midnight", () => {
    expect(untilNextDay(new Date(2026, 9, 12, 23, 59, 0))).toBe(2 * 60_000);
    expect(untilNextDay(new Date(2026, 9, 12, 0, 0, 30))).toBe(24 * 3_600_000 + 30_000);
  });
});
