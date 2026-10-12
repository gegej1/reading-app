// Answers (回答): what the user says to a pending ruling (Q-001), and the
// reason when they reject an AI ruling on the audit checklist (不认可). Each
// is one new file in inbox/裁决/, in the format of the knowledge base's
// AGENTS.md section 三: YAML with 对象, 时间 and 来源, then the user's words
// verbatim. 落裁决 later turns them into precedents, 底色 drafts or new Q files.

import {
  assertSource,
  fileStamp,
  isBlankCapture,
  isoLocal,
  localTime,
  verbatimBody,
  type Capture,
  type CaptureSource,
  type LocalTime,
} from "./capture";
import { fmString, parseFrontmatter } from "./frontmatter";

/** The folder 落裁决 works through. */
export const ANSWER_DIR = "inbox/裁决";

/** What an answer is about. */
export type AnswerTarget =
  /** A pending ruling, `对象: Q-001`. */
  | { kind: "ruling"; id: string }
  /** A rejected audit item, `对象: 抽查 2026-10-12-入库报告 I-12`: report names make item ids unique. */
  | { kind: "audit"; report: string; itemId: string };

const RULING_ID = /^Q-\d+$/;
const ITEM_ID = /^I-\d+$/;
const AUDIT_PREFIX = "抽查";

/** The `对象` value of a target. */
export function answerObject(target: AnswerTarget): string {
  return target.kind === "ruling" ? target.id : `${AUDIT_PREFIX} ${target.report} ${target.itemId}`;
}

/**
 * Reads an `对象` value: `Q-001` (also `Q-001-标题`), or `抽查 <report> I-12`.
 * Null for anything else.
 */
export function parseAnswerObject(object: string | null): AnswerTarget | null {
  const text = object?.trim() ?? "";
  const ruling = /^(Q-\d+)(?![0-9A-Za-z])/.exec(text);
  if (ruling?.[1]) return { kind: "ruling", id: ruling[1] };
  if (!text.startsWith(AUDIT_PREFIX)) return null;
  const words = text.slice(AUDIT_PREFIX.length).trim().split(/\s+/);
  const itemId = words.pop();
  const report = words.join(" ");
  if (!itemId || !ITEM_ID.test(itemId) || report === "") return null;
  return { kind: "audit", report, itemId };
}

/** The target of one queued answer file, from its frontmatter. */
export function answerTargetOf(content: string): AnswerTarget | null {
  return parseAnswerObject(fmString(parseFrontmatter(content).data, "对象"));
}

/** A YAML value: plain when that is safe, else double-quoted. */
function yamlValue(text: string): string {
  const plain = !/^[\s\-?:,[\]{}#&*!|>'"%@`]/.test(text) && !/: |\s#|:$|[\n\r\t"\\]/.test(text) && text.trim() === text;
  return plain ? text : JSON.stringify(text);
}

/** A report name that can be part of a file name in inbox/裁决/. */
function assertReportName(report: string): void {
  if (report === "" || /[/\\\s]/.test(report) || report.startsWith(".") || report.endsWith(".md")) {
    throw new Error(`入库报告的名字没法用在文件名里：${report}`);
  }
}

function assertTarget(target: AnswerTarget): void {
  if (target.kind === "ruling") {
    if (!RULING_ID.test(target.id)) throw new Error(`不是待裁决的编号：${target.id}`);
    return;
  }
  assertReportName(target.report);
  if (!ITEM_ID.test(target.itemId)) throw new Error(`不是抽查条目的编号：${target.itemId}`);
}

/**
 * Builds the file for one answer made at `at`:
 *
 * - a ruling: `inbox/裁决/Q-001-2026-10-12T14-03-22-mac.md`, commit message
 *   `收件箱：回答 Q-001（mac）`;
 * - a rejected audit item: `inbox/裁决/抽查-2026-10-12-入库报告-I-12-2026-10-12T14-03-22-mac.md`,
 *   commit message `收件箱：驳回抽查 I-12（2026-10-12-入库报告，mac）`.
 *
 * The text is kept verbatim like a capture's (CRLF becomes LF, exactly one
 * newline at the end). Throws for empty text, unknown sources and targets
 * that cannot be named.
 */
export function buildAnswerAt({
  target,
  text,
  source,
  at,
}: {
  target: AnswerTarget;
  text: string;
  source: CaptureSource;
  at: LocalTime;
}): Capture {
  assertSource(source);
  assertTarget(target);
  if (isBlankCapture(text)) throw new Error("还没写字，没有可以记下的回答");
  const stamp = fileStamp(at);
  const name =
    target.kind === "ruling"
      ? `${target.id}-${stamp}-${source}`
      : `${AUDIT_PREFIX}-${target.report}-${target.itemId}-${stamp}-${source}`;
  const message =
    target.kind === "ruling"
      ? `收件箱：回答 ${target.id}（${source}）`
      : `收件箱：驳回抽查 ${target.itemId}（${target.report}，${source}）`;
  return {
    path: `${ANSWER_DIR}/${name}.md`,
    content: `---\n对象: ${yamlValue(answerObject(target))}\n时间: ${isoLocal(at)}\n来源: ${source}\n---\n\n${verbatimBody(text)}`,
    message,
  };
}

/** {@link buildAnswerAt} for an answer made now (local time of `now`). */
export function buildAnswer(args: { target: AnswerTarget; text: string; source: CaptureSource; now: Date }): Capture {
  return buildAnswerAt({ target: args.target, text: args.text, source: args.source, at: localTime(args.now) });
}

/** One queued answer: its file, what it is about, and the user's words. */
export interface QueuedAnswer {
  path: string;
  target: AnswerTarget;
  /** The `时间` value, as written. */
  time: string | null;
  /** The answer without the frontmatter, trimmed. */
  text: string;
}

/** The queued answers by what they are about. */
export interface AnswerIndex {
  /** Q id → its answers, oldest first. */
  rulings: Map<string, QueuedAnswer[]>;
  /** {@link auditKey} → the rejections of that item, oldest first. */
  audits: Map<string, QueuedAnswer[]>;
  /** Answer files whose 对象 could not be read. */
  unknown: string[];
}

/** The key of an audit item in {@link AnswerIndex.audits}. */
export function auditKey(report: string, itemId: string): string {
  return `${report}\u0000${itemId}`;
}

/** Indexes the files in inbox/裁决/ (sorted by path, so by time for the app's own names). */
export function indexAnswers(files: readonly { path: string; content: string }[]): AnswerIndex {
  const index: AnswerIndex = { rulings: new Map(), audits: new Map(), unknown: [] };
  const sorted = [...files].sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  for (const file of sorted) {
    const { data, body } = parseFrontmatter(file.content);
    const target = parseAnswerObject(fmString(data, "对象"));
    if (!target) {
      index.unknown.push(file.path);
      continue;
    }
    const answer: QueuedAnswer = { path: file.path, target, time: fmString(data, "时间"), text: body.trim() };
    const [map, key] =
      target.kind === "ruling" ? [index.rulings, target.id] : [index.audits, auditKey(target.report, target.itemId)];
    const list = map.get(key);
    if (list) list.push(answer);
    else map.set(key, [answer]);
  }
  return index;
}
