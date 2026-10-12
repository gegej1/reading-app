import { describe, expect, it } from "vitest";
import { parseReport } from "../../domain";
import { fixtureKb } from "../../domain/__fixtures__";
import { editKbFile } from "../../app/editFile";
import { createMemoryStore } from "../../store/memoryStore";
import { auditEdit, auditMessage } from "./audit";

const PATH = "reports/2026-09-08-入库报告-2.md";
const NAME = "2026-09-08-入库报告-2";
const ORIGINAL = fixtureKb[PATH] ?? "";

const checkedIn = (text: string, id: string) => parseReport(PATH, text).auditItems.find((i) => i.itemId === id)?.checked;

async function tick(store: ReturnType<typeof createMemoryStore>, id: string, desired: boolean) {
  const { edit, message } = auditEdit(id, NAME, desired);
  return editKbFile(store, PATH, edit, message);
}

describe("auditMessage", () => {
  it("names the item and its report", () => {
    expect(auditMessage("I-12", "2026-10-12-入库报告", true)).toBe("抽查：认可 I-12（2026-10-12-入库报告）");
    expect(auditMessage("I-12", "2026-10-12-入库报告", false)).toBe("抽查：取消认可 I-12（2026-10-12-入库报告）");
  });
});

describe("auditEdit through editKbFile", () => {
  it("ticks one item, commits it with 抽查：认可 and changes only that line", async () => {
    const store = createMemoryStore(fixtureKb);
    const result = await tick(store, "I-07", true);
    expect(result).toMatchObject({ changed: true, message: "抽查：认可 I-07（2026-09-08-入库报告-2）" });
    expect(store.commits).toEqual([{ path: PATH, message: "抽查：认可 I-07（2026-09-08-入库报告-2）" }]);
    const saved = await store.read(PATH);
    expect(checkedIn(saved, "I-07")).toBe(true);
    const before = ORIGINAL.split("\n");
    expect(saved.split("\n").filter((line, i) => line !== before[i])).toEqual([
      "- [x] I-07 周五下午不安排会议 ↔ 周会：不冲突，周会在周一（依据 [[P-001-速度与退路]]）",
    ]);
  });

  it("writes nothing when the report already says so (a stale screen)", async () => {
    const store = createMemoryStore(fixtureKb);
    expect((await tick(store, "I-08", true)).changed).toBe(false);
    expect((await tick(store, "I-07", false)).changed).toBe(false);
    expect(store.commits).toEqual([]);
  });

  it("un-ticks with 抽查：取消认可, and ticking again restores the line", async () => {
    const store = createMemoryStore(fixtureKb);
    await tick(store, "I-08", false);
    expect(checkedIn(await store.read(PATH), "I-08")).toBe(false);
    await tick(store, "I-08", true);
    expect(store.commits.map((c) => c.message)).toEqual([
      "抽查：取消认可 I-08（2026-09-08-入库报告-2）",
      "抽查：认可 I-08（2026-09-08-入库报告-2）",
    ]);
    // The item was written as [X]; it comes back as [x], the only difference.
    expect(await store.read(PATH)).toBe(ORIGINAL.replace("- [X] I-08", "- [x] I-08"));
  });

  it("refuses items outside the audit section", async () => {
    const store = createMemoryStore(fixtureKb);
    await expect(tick(store, "I-99", true)).rejects.toThrow(/I-99/);
    expect(store.commits).toEqual([]);
  });
});
