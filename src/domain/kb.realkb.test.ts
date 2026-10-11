// Local-only compatibility check against the real knowledge base.
//
// Runs only when the KB folder exists (DISE_KB_PATH or ~/Desktop/GHJProject/阅读)
// and CI is not set; it is skipped everywhere else. It only reads through
// node fs and never writes. The KB grows with every ingest, so counts are
// minimums. Run it alone with `pnpm test:kb`.

import { existsSync } from "node:fs";
import { beforeAll, describe, expect, it } from "vitest";
import { listKbFiles, readKbFile, resolveDevKbRoot } from "../../dev/kbFs";
import { loadKb, type KbModel } from "./kb";

const root = resolveDevKbRoot();
const enabled = !process.env.CI && existsSync(root);
const FIRST_REPORT = "reports/2026-10-11-首次入库报告.md";

describe.skipIf(!enabled)(`real KB (read-only): ${root}`, () => {
  let model: KbModel;

  beforeAll(async () => {
    model = await loadKb({
      list: () => listKbFiles(root),
      read: (path) => readKbFile(root, path),
    });
  });

  it("parses 底色 entries D01–D13", () => {
    const ids = model.dise?.entries.map((entry) => entry.id) ?? [];
    for (let n = 1; n <= 13; n += 1) {
      expect(ids).toContain(`D${String(n).padStart(2, "0")}`);
    }
    for (const entry of model.dise?.entries ?? []) {
      expect(entry.title, entry.id).not.toBe("");
      expect(entry.quote, entry.id).not.toBe("");
    }
  });

  it("parses at least 5 判例 with ids and titles", () => {
    expect(model.precedents.length).toBeGreaterThanOrEqual(5);
    for (const precedent of model.precedents) {
      expect(precedent.id, precedent.path).toMatch(/^P-\d{3}$/);
      expect(precedent.title, precedent.path).not.toBe("");
    }
  });

  it("parses at least 7 待裁决 Q files with a priority", () => {
    expect(model.rulings.length).toBeGreaterThanOrEqual(7);
    for (const ruling of model.rulings) {
      expect(ruling.id, ruling.path).toMatch(/^Q-\d{3}$/);
      expect(["高", "中", "低"], ruling.path).toContain(ruling.priority);
    }
  });

  it("finds at least 3 audit items in the first ingest report", () => {
    const report = model.reports.find((r) => r.path === FIRST_REPORT);
    expect(report, FIRST_REPORT).toBeDefined();
    expect(report?.resultLine).not.toBeNull();
    const items = report?.auditItems ?? [];
    expect(items.length).toBeGreaterThanOrEqual(3);
    for (const item of items) {
      expect(item.itemId, item.text).toMatch(/^I-\d+$/);
      expect(item.precedent, item.text).toMatch(/^P-\d{3}/);
    }
  });

  it("has no broken wikilinks outside backticks", () => {
    const broken = model.links.broken.map(
      (b) => `${b.from}:${b.link.line + 1} ${b.link.raw}`,
    );
    expect(broken).toEqual([]);
  });
});
