// Tests of the shortcut generator (scripts/shortcuts/definitions.mjs): the
// checked-in sources are up to date, every variable points at an action that
// exists before it, every path stays inside the bridge folder, and on macOS
// the property lists are valid for plutil.

import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { allShortcuts, stableUuid, tokenText, toPlist } from "./shortcuts/definitions.mjs";

const root = join(import.meta.dirname, "..");

/** Every value of a nested structure. */
function* walk(value) {
  yield value;
  if (Array.isArray(value)) for (const item of value) yield* walk(item);
  else if (value && typeof value === "object") for (const item of Object.values(value)) yield* walk(item);
}

describe("the iPhone shortcuts", () => {
  const shortcuts = allShortcuts();

  it("are the three shortcuts, checked in up to date", () => {
    expect(Object.keys(shortcuts)).toEqual(["记一笔", "看底色", "答裁决"]);
    for (const [name, shortcut] of Object.entries(shortcuts)) {
      const checkedIn = readFileSync(join(root, "shortcuts", `${name}.plist`), "utf8");
      expect(checkedIn, `shortcuts/${name}.plist is out of date: run node scripts/build-shortcuts.mjs`).toBe(
        toPlist(shortcut),
      );
    }
  });

  it("only refer to outputs of earlier actions", () => {
    for (const [name, shortcut] of Object.entries(shortcuts)) {
      const defined = new Set();
      for (const action of shortcut.WFWorkflowActions) {
        const params = action.WFWorkflowActionParameters;
        for (const value of walk(params)) {
          if (value && typeof value === "object" && value.Type === "ActionOutput") {
            expect(defined.has(value.OutputUUID), `${name}: ${action.WFWorkflowActionIdentifier}`).toBe(true);
          }
        }
        if (params.UUID) {
          expect(defined.has(params.UUID), `${name}: duplicate UUID`).toBe(false);
          defined.add(params.UUID);
        }
      }
    }
  });

  it("read and write only inside the bridge folder", () => {
    const paths = [];
    for (const shortcut of Object.values(shortcuts)) {
      for (const action of shortcut.WFWorkflowActions) {
        const params = action.WFWorkflowActionParameters;
        for (const key of ["WFGetFilePath", "WFFileDestinationPath"]) {
          const value = params[key];
          if (value === undefined) continue;
          paths.push(typeof value === "string" ? value : value.Value.string);
        }
      }
    }
    expect(paths.length).toBeGreaterThanOrEqual(10);
    for (const path of paths) {
      expect(path).toMatch(/^\/?底色\/(阅读|收件\/记一笔|收件\/回答)\//);
      expect(path).not.toContain("..");
    }
    const saves = paths.filter((path) => path.startsWith("/"));
    expect(saves).toEqual(["/底色/收件/记一笔/", "/底色/收件/回答/"]);
  });

  it("name files with an ISO 8601 stamp, whatever calendar the phone uses", () => {
    for (const name of ["记一笔", "答裁决"]) {
      const actions = shortcuts[name].WFWorkflowActions;
      const id = (action) => action.WFWorkflowActionIdentifier;
      const params = (action) => action.WFWorkflowActionParameters;
      const dates = actions.filter((action) => id(action) === "is.workflow.actions.format.date");
      expect(dates.length, name).toBe(1);
      const date = params(dates[0]);
      // A custom pattern follows the phone's calendar (2569 in the Buddhist
      // one); the ISO 8601 style is always Gregorian.
      expect(date.WFDateFormatStyle).toBe("ISO 8601");
      expect(date.WFISO8601IncludeTime).toBe(true);
      expect(date.WFDateFormat).toBeUndefined();
      // Colons become dashes: 2026-10-12T14:03:22+08:00 -> 2026-10-12T14-03-22+08-00.
      const replace = actions.find((action) => id(action) === "is.workflow.actions.text.replace");
      expect(params(replace)).toMatchObject({ WFReplaceTextFind: ":", WFReplaceTextReplace: "-", WFReplaceTextRegularExpression: false });
      expect(params(replace).WFInput.Value.attachmentsByRange["{0, 1}"].OutputUUID).toBe(date.UUID);
      const naming = params(actions.find((action) => id(action) === "is.workflow.actions.setitemname"));
      const refs = Object.values(naming.WFName.Value.attachmentsByRange).map((ref) => ref.OutputUUID);
      expect(refs).toContain(params(replace).UUID);
      expect(refs).not.toContain(date.UUID);
    }
  });

  it("are the capture shortcut in the share sheet", () => {
    const capture = shortcuts["记一笔"];
    expect(capture.WFWorkflowTypes).toEqual(["ActionExtension"]);
    expect(capture.WFWorkflowNoInputBehavior.Name).toBe("WFWorkflowNoInputBehaviorAskForInput");
    const last = capture.WFWorkflowActions.at(-1);
    expect(last.WFWorkflowActionParameters.Text).toBe("已记下");
  });

  it("place variables at UTF-16 offsets and derive stable ids", () => {
    const ref = { Type: "CurrentDate" };
    const token = tokenText("底色/", ref, ".txt");
    expect(token.Value.string).toBe("底色/￼.txt");
    expect(Object.keys(token.Value.attachmentsByRange)).toEqual(["{3, 1}"]);
    expect(stableUuid("a")).toBe(stableUuid("a"));
    expect(stableUuid("a")).not.toBe(stableUuid("b"));
    expect(stableUuid("a")).toMatch(/^[0-9A-F]{8}-[0-9A-F]{4}-4[0-9A-F]{3}-8[0-9A-F]{3}-[0-9A-F]{12}$/);
  });

  it("contain nothing personal", () => {
    for (const shortcut of Object.values(shortcuts)) {
      const xml = toPlist(shortcut);
      expect(xml).not.toMatch(/\/Users\/|gegej1|eduardo/i);
    }
  });

  it.runIf(process.platform === "darwin")("are valid property lists", () => {
    const dir = mkdtempSync(join(tmpdir(), "dise-plist-"));
    try {
      for (const [name, shortcut] of Object.entries(shortcuts)) {
        const file = join(dir, `${name}.plist`);
        writeFileSync(file, toPlist(shortcut));
        const result = spawnSync("/usr/bin/plutil", ["-lint", file], { encoding: "utf8" });
        expect(result.status, result.stdout + result.stderr).toBe(0);
      }
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
