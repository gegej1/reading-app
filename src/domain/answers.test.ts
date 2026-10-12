import { describe, expect, it } from "vitest";
import {
  answerObject,
  answerTargetOf,
  auditKey,
  buildAnswer,
  buildAnswerAt,
  indexAnswers,
  parseAnswerObject,
  type AnswerTarget,
} from "./answers";
import { saveCapture, type LocalTime } from "./capture";
import { parseFrontmatter } from "./frontmatter";
import { queueBody } from "./gate";
import { buildKbModel } from "./kb";
import { fixtureKb } from "./__fixtures__";

const at: LocalTime = { year: 2026, month: 10, day: 12, hour: 14, minute: 3, second: 22, offsetMinutes: 480 };
const q: AnswerTarget = { kind: "ruling", id: "Q-001" };
const audit: AnswerTarget = { kind: "audit", report: "2026-10-12-入库报告", itemId: "I-12" };

describe("buildAnswerAt", () => {
  it("writes an answer to a pending ruling into inbox/裁决/", () => {
    const answer = buildAnswerAt({ target: q, text: "先不取消，改成两周一次。", source: "mac", at });
    expect(answer).toEqual({
      path: "inbox/裁决/Q-001-2026-10-12T14-03-22-mac.md",
      content: "---\n对象: Q-001\n时间: 2026-10-12T14:03:22+08:00\n来源: mac\n---\n\n先不取消，改成两周一次。\n",
      message: "收件箱：回答 Q-001（mac）",
    });
  });

  it("writes the reason of a rejected audit item, naming the report", () => {
    const rejection = buildAnswerAt({ target: audit, text: "这条判错了。", source: "web", at: { ...at, offsetMinutes: -330 } });
    expect(rejection).toEqual({
      path: "inbox/裁决/抽查-2026-10-12-入库报告-I-12-2026-10-12T14-03-22-web.md",
      content: "---\n对象: 抽查 2026-10-12-入库报告 I-12\n时间: 2026-10-12T14:03:22-05:30\n来源: web\n---\n\n这条判错了。\n",
      message: "收件箱：驳回抽查 I-12（2026-10-12-入库报告，web）",
    });
  });

  it("keeps the words verbatim: only CRLF becomes LF and there is exactly one newline at the end", () => {
    const text = "  第一行，前面的空格留着\r\n\r\n第二段  \r\n\r\n\r\n";
    const { content } = buildAnswerAt({ target: q, text, source: "mac", at });
    expect(content.endsWith("\n\n  第一行，前面的空格留着\n\n第二段  \n")).toBe(true);
    // What the gate later looks for in raw/ is exactly the text.
    expect(queueBody(content)).toBe("第一行，前面的空格留着\n\n第二段");
  });

  it("produces frontmatter that reads back as written", () => {
    for (const target of [q, audit]) {
      const { content } = buildAnswerAt({ target, text: "原话", source: "mac", at });
      const { data, error } = parseFrontmatter(content);
      expect(error).toBeNull();
      expect(data).toEqual({ 对象: answerObject(target), 时间: "2026-10-12T14:03:22+08:00", 来源: "mac" });
      expect(answerTargetOf(content)).toEqual(target);
    }
    // A report name YAML would misread is quoted.
    const odd = buildAnswerAt({ target: { kind: "audit", report: "报告#1", itemId: "I-01" }, text: "x", source: "mac", at });
    expect(parseFrontmatter(odd.content).data["对象"]).toBe("抽查 报告#1 I-01");
  });

  it("refuses empty text, unknown sources and targets that cannot be named", () => {
    expect(() => buildAnswerAt({ target: q, text: " \n ", source: "mac", at })).toThrow(/还没写字/);
    expect(() => buildAnswerAt({ target: q, text: "x", source: "phone" as never, at })).toThrow(/来源/);
    expect(() => buildAnswerAt({ target: { kind: "ruling", id: "Q-1a" }, text: "x", source: "mac", at })).toThrow();
    expect(() => buildAnswerAt({ target: { ...audit, itemId: "12" } as AnswerTarget, text: "x", source: "mac", at })).toThrow();
    for (const report of ["", "../x", "a/b", ".hidden", "有 空格", "a.md"]) {
      expect(() => buildAnswerAt({ target: { ...audit, report } as AnswerTarget, text: "x", source: "mac", at }), report).toThrow();
    }
  });

  it("uses the local time of now", () => {
    const answer = buildAnswer({ target: q, text: "好", source: "mac", now: new Date(2026, 9, 12, 9, 5, 7) });
    expect(answer.path).toBe("inbox/裁决/Q-001-2026-10-12T09-05-07-mac.md");
  });

  it("gets the next free name when two answers fall into the same second", async () => {
    const created: string[] = [];
    const target = {
      async create(path: string) {
        if (created.includes(path)) throw Object.assign(new Error("exists"), { code: "exists" });
        created.push(path);
      },
    };
    const answer = buildAnswerAt({ target: q, text: "一", source: "mac", at });
    await saveCapture(target, answer);
    const second = await saveCapture(target, answer);
    expect(second.path).toBe("inbox/裁决/Q-001-2026-10-12T14-03-22-mac-2.md");
  });
});

describe("parseAnswerObject", () => {
  it("reads rulings and audit items, and nothing else", () => {
    expect(parseAnswerObject("Q-004")).toEqual({ kind: "ruling", id: "Q-004" });
    expect(parseAnswerObject(" Q-004-工具要不要统一 ")).toEqual({ kind: "ruling", id: "Q-004" });
    expect(parseAnswerObject("抽查 2026-10-11-入库报告 I-02")).toEqual({
      kind: "audit",
      report: "2026-10-11-入库报告",
      itemId: "I-02",
    });
    for (const bad of [null, "", "Q-", "Q-12x", "抽查", "抽查 I-02", "抽查 报告 12", "随便写的"]) {
      expect(parseAnswerObject(bad), String(bad)).toBeNull();
    }
  });
});

describe("indexAnswers and the model", () => {
  it("groups queued answers by ruling and by audit item, oldest first", () => {
    const index = indexAnswers([
      { path: "inbox/裁决/Q-001-2026-10-12T15-00-00-mac.md", content: "---\n对象: Q-001\n时间: b\n---\n\n第二次\n" },
      { path: "inbox/裁决/Q-001-2026-10-12T14-00-00-mac.md", content: "---\n对象: Q-001\n时间: a\n---\n\n第一次\n" },
      { path: "inbox/裁决/抽查-r-I-01-x.md", content: "---\n对象: 抽查 r I-01\n---\n\n理由\n" },
      { path: "inbox/裁决/乱写.md", content: "没有对象\n" },
    ]);
    expect(index.rulings.get("Q-001")?.map((answer) => answer.text)).toEqual(["第一次", "第二次"]);
    expect(index.rulings.get("Q-001")?.[0]?.time).toBe("a");
    expect(index.audits.get(auditKey("r", "I-01"))?.[0]?.text).toBe("理由");
    expect(index.unknown).toEqual(["inbox/裁决/乱写.md"]);
  });

  it("is part of the model", () => {
    const model = buildKbModel(Object.entries(fixtureKb).map(([path, content]) => ({ path, content })));
    expect([...model.answerIndex.rulings.keys()]).toEqual(["Q-001"]);
    expect(model.dashboard.answeredRulings).toBe(1);
  });
});
