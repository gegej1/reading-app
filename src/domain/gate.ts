// The gate: a deterministic check of what an AI run (入库 or 落裁决) changed,
// before the app commits anything. It implements the checks of section 九
// of the knowledge base's AGENTS.md.
//
// Pure: the desktop app collects the run's changes with git and passes them
// in; nothing here reads files or runs commands. The Rust side re-checks the
// two cheapest rules (allowed paths, raw/ only grows) at commit time.

import { DISE_PATH } from "./kb";
import { diseEntryTexts, DRAFT_MARKER_TEXT, parseDise, type DiseEntryText } from "./dise";
import { fmString, parseFrontmatter, splitFrontmatter } from "./frontmatter";
import { precedentRulingTexts } from "./precedents";
import { parseRuling, type Ruling } from "./rulings";
import { analyzeLinks, buildLinkIndex, type LinkRef } from "./wikilinks";

/** The two kinds of runs the app starts. */
export type RunTask = "ingest" | "landRulings";

/** One file the run added, changed or removed (compared with the commit the run started from). */
export interface GateFile {
  /** Repo-relative POSIX path. */
  path: string;
  status: "added" | "modified" | "deleted";
  /** Text before the run; null when the file did not exist, or for an opaque file. */
  before: string | null;
  /** Text after the run; null when the file is gone, or for an opaque file. */
  after: string | null;
  /** Not comparable as text: a binary file, a symlink, or too large. */
  opaque: "binary" | "symlink" | "large" | null;
}

/** A file with its content. */
export interface GateDoc {
  path: string;
  content: string;
}

/** A commit that appeared during the run and that the app did not make. */
export interface GateCommit {
  hash: string;
  subject: string;
}

export interface GateInput {
  task: RunTask;
  /** Local date of the run, `YYYY-MM-DD`. */
  date: string;
  /** Everything the run changed, queue-file deletions included. Captures the app saved meanwhile are not in here. */
  changes: GateFile[];
  /** Every other Markdown file of the knowledge base, as it is now. */
  unchanged: GateDoc[];
  /** The queue files as they were when the run started. */
  queue: GateDoc[];
  /** The engine's final message (its last reply). */
  finalMessage: string | null;
  /** Why the engine did not finish normally (error, timeout, cancel); null when it did. */
  engineFailure: string | null;
  /** Commits made by something other than the app while the engine ran. */
  foreignCommits: GateCommit[];
  /** Files outside git (untracked before the run) that the run changed or removed. */
  untrackedChanged: string[];
}

export type GateRule =
  | "paths"
  | "fileKind"
  | "rawAppendOnly"
  | "queueVerbatim"
  | "confirmedDise"
  | "precedentRuling"
  | "brokenLinks"
  | "report"
  | "rulingsAnswered"
  | "engineCommits"
  | "engine"
  | "untracked"
  | "finalLine"
  /** The gate itself could not finish (reported by its host, never by `runGate`). */
  | "gate";

/** One problem, in plain Chinese, naming the file when there is one. */
export interface GateIssue {
  rule: GateRule;
  path: string | null;
  message: string;
}

/** Why a change waits for the user's approval (AGENTS.md section 九, "需要我确认的改动"). */
export type ApprovalRule =
  /** An existing 底色 entry changed or went away (confirmed or not). */
  | "diseEntry"
  /** A new 底色 entry without the draft marker. */
  | "diseDraft"
  /** The frontmatter, the preface or the section headings of the 底色. */
  | "diseFrame"
  /** Anything under systems/. */
  | "systems";

/**
 * A change that breaks no rule but touches the user's own layer: the app
 * stops and shows it, and only commits once the user agreed.
 */
export interface GateApproval {
  rule: ApprovalRule;
  path: string;
  /** The 底色 entry (`D09`), when it is about one. */
  entry: string | null;
  /** In plain Chinese: what changed and why it waits for the user. */
  message: string;
  /** The part that changed, before and after (null: did not exist, or the whole file is meant). */
  before: string | null;
  after: string | null;
}

export interface GateReport {
  /** No violations and nothing to approve: the app may commit. */
  ok: boolean;
  violations: GateIssue[];
  /** Worth knowing, but no reason to hold the commit back. */
  warnings: GateIssue[];
  /** No rule broken, but the user decides: the app commits only after they agree. */
  approvals: GateApproval[];
  /** Every changed path the commit takes (never a queue file). */
  commitPaths: string[];
  /** Queue files whose body is verbatim in raw/: the app deletes them in the same commit. */
  deleteQueuePaths: string[];
  /** Queue files the engine deleted or edited although their body is not in raw/: the app puts them back. */
  restoreQueuePaths: string[];
  /** One line about the outcome. */
  summary: string;
  /** The engine's final JSON line, when it gave one. */
  final: Record<string, unknown> | null;
}

const ALLOWED_FOLDERS = ["raw/", "wiki/", "systems/", "self/", "reports/"] as const;
const ALLOWED_FILES = ["index.md", "log.md"] as const;
const ALLOWED_TEXT = "只能改 raw/、wiki/、systems/、self/、reports/ 和 index.md、log.md";
const RULING_FILE = /^self\/待裁决\/[^/]+\.md$/;
const PRECEDENT_FILE = /^self\/判例\/.+\.md$/;
const RULING_SECTION = '"## 裁决"';

const byCodeUnit = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);
const isMarkdown = (path: string) => path.endsWith(".md");

/** True for the paths an AI run may change (AGENTS.md section 九). */
export function isAllowedPath(path: string): boolean {
  if ((ALLOWED_FILES as readonly string[]).includes(path)) return true;
  return ALLOWED_FOLDERS.some((folder) => path.startsWith(folder) && path.length > folder.length);
}

/** Line endings as LF. */
function normalizeNewlines(text: string): string {
  return text.replace(/\r\n?/g, "\n");
}

/** The part of a queue file that must reach raw/ verbatim: the body without frontmatter, LF line endings, trimmed. */
export function queueBody(content: string): string {
  return normalizeNewlines(splitFrontmatter(content).body).trim();
}

/**
 * The engine's final JSON line (`{"任务": "入库", …}`): the last line of its
 * reply, also when wrapped in a code fence or spread over several lines at
 * the end. Null when there is no JSON object.
 */
export function parseFinalLine(message: string): Record<string, unknown> | null {
  const lines = normalizeNewlines(message)
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line !== "" && !line.startsWith("```"));
  const candidates: string[] = [];
  const last = lines[lines.length - 1];
  if (last !== undefined) candidates.push(last.replace(/^`+|`+$/g, ""));
  for (let i = lines.length - 1; i >= 0; i -= 1) {
    if ((lines[i] ?? "").startsWith("{")) {
      candidates.push(lines.slice(i).join("\n"));
      break;
    }
  }
  for (const candidate of candidates) {
    try {
      const value: unknown = JSON.parse(candidate);
      if (typeof value === "object" && value !== null && !Array.isArray(value)) {
        return value as Record<string, unknown>;
      }
    } catch {
      // Not this one.
    }
  }
  return null;
}

/** The knowledge base on one side of the run: which paths exist, and the text of those that are text. */
interface Side {
  paths: Set<string>;
  text: Map<string, string>;
}

function sides(input: GateInput): { before: Side; after: Side } {
  const before: Side = { paths: new Set(), text: new Map() };
  const after: Side = { paths: new Set(), text: new Map() };
  for (const doc of input.unchanged) {
    for (const side of [before, after]) {
      side.paths.add(doc.path);
      side.text.set(doc.path, doc.content);
    }
  }
  for (const change of input.changes) {
    if (change.status !== "added") {
      before.paths.add(change.path);
      if (change.before !== null) before.text.set(change.path, change.before);
    }
    if (change.status !== "deleted") {
      after.paths.add(change.path);
      if (change.after !== null) after.text.set(change.path, change.after);
    }
  }
  return { before, after };
}

function brokenLinks(side: Side): LinkRef[] {
  const paths = [...side.paths].filter(isMarkdown);
  const docs = [...side.text.entries()]
    .filter(([path]) => isMarkdown(path))
    .map(([path, content]) => ({ path, content }));
  return analyzeLinks(docs, buildLinkIndex(paths)).broken;
}

/**
 * Broken links that the run added. Counted per target, so a file that was
 * moved together with an old broken link does not count; links that were
 * broken before and still are do not count either.
 */
function newBrokenLinks(before: Side, after: Side): LinkRef[] {
  const old = brokenLinks(before);
  const now = brokenLinks(after);
  const count = (refs: LinkRef[]) => {
    const counts = new Map<string, number>();
    for (const ref of refs) counts.set(ref.link.target, (counts.get(ref.link.target) ?? 0) + 1);
    return counts;
  };
  const oldCounts = count(old);
  const nowCounts = count(now);
  const oldPairs = new Map<string, number>();
  for (const ref of old) {
    const key = `${ref.from}\u0000${ref.link.target}`;
    oldPairs.set(key, (oldPairs.get(key) ?? 0) + 1);
  }
  const fresh: LinkRef[] = [];
  for (const ref of now) {
    if ((nowCounts.get(ref.link.target) ?? 0) <= (oldCounts.get(ref.link.target) ?? 0)) continue;
    const key = `${ref.from}\u0000${ref.link.target}`;
    const left = oldPairs.get(key) ?? 0;
    if (left > 0) oldPairs.set(key, left - 1);
    else fresh.push(ref);
  }
  return fresh;
}

const OPAQUE_TEXT: Record<NonNullable<GateFile["opaque"]>, string> = {
  binary: "不是文本文件",
  symlink: "是符号链接",
  large: "文件太大",
};

/** Checks one run. Every violation is listed, so one look shows all problems. */
export function runGate(input: GateInput): GateReport {
  const violations: GateIssue[] = [];
  const warnings: GateIssue[] = [];
  const approvals: GateApproval[] = [];
  const violation = (rule: GateRule, path: string | null, message: string) => violations.push({ rule, path, message });
  const warning = (rule: GateRule, path: string | null, message: string) => warnings.push({ rule, path, message });

  const ingest = input.task === "ingest";
  const queuePaths = new Set(input.queue.map((file) => file.path));
  const changes = new Map(input.changes.map((change) => [change.path, change]));
  const { before, after } = sides(input);
  const queueBodies = input.queue.map((file) => queueBody(file.content));

  // The engine itself.
  if (input.engineFailure !== null) {
    violation("engine", null, `AI 没有正常结束：${input.engineFailure}`);
  }
  for (const commit of input.foreignCommits) {
    violation(
      "engineCommits",
      null,
      `运行期间多了一个不是 App 做的提交 ${commit.hash.slice(0, 10)}（AI 不能自己做 git 操作）`,
    );
  }
  for (const path of input.untrackedChanged) {
    violation("untracked", path, `${path} 不在 git 里，运行中被改动或删除了；App 不会提交它，也不会替你恢复`);
  }

  // Only the allowed folders; queue files may only disappear (the app checks them below).
  for (const change of input.changes) {
    if (queuePaths.has(change.path)) {
      if (change.status !== "deleted") {
        violation("paths", change.path, `改动了队列文件 ${change.path}（队列文件只能原样留着，由 App 核对后删除）`);
      }
      continue;
    }
    if (!isAllowedPath(change.path)) {
      violation("paths", change.path, `改动了不允许改的文件：${change.path}（${ALLOWED_TEXT}）`);
    }
    if (change.opaque === "symlink" && change.status !== "deleted") {
      violation("fileKind", change.path, `${change.path} 是符号链接，App 不会提交符号链接`);
    }
  }

  // raw/ only grows at the end.
  for (const change of input.changes) {
    if (!change.path.startsWith("raw/") || change.status === "added") continue;
    if (change.status === "deleted") {
      violation("rawAppendOnly", change.path, `删除了原料 ${change.path}（原料只增不改，也不能改名）`);
    } else if (change.opaque !== null || change.before === null || change.after === null) {
      const why = change.opaque ? OPAQUE_TEXT[change.opaque] : "内容读不出来";
      violation("rawAppendOnly", change.path, `没法核对原料 ${change.path} 是不是只在末尾追加（${why}）`);
    } else if (!change.after.startsWith(change.before)) {
      violation("rawAppendOnly", change.path, `原料 ${change.path} 的原有内容被改动了（只能在末尾追加）`);
    }
  }

  // Every queue body is verbatim in raw/.
  const rawTexts = [...after.text.entries()]
    .filter(([path]) => path.startsWith("raw/"))
    .map(([, text]) => normalizeNewlines(text));
  const deleteQueuePaths: string[] = [];
  const restoreQueuePaths: string[] = [];
  const kind = ingest ? "随手记" : "裁决文件";
  const target = ingest ? `raw/inbox/${input.date}-收件箱.md` : `raw/对话/${input.date}-裁决.md`;
  input.queue.forEach((file, index) => {
    const body = queueBodies[index] ?? "";
    if (rawTexts.some((text) => text.includes(body))) {
      deleteQueuePaths.push(file.path);
      return;
    }
    const change = changes.get(file.path);
    if (change) {
      restoreQueuePaths.push(file.path);
      const what = change.status === "deleted" ? "AI 却把它删掉了" : "文件还被改了";
      violation(
        "queueVerbatim",
        file.path,
        `${kind} ${file.path} 的正文没有逐字出现在 raw/ 里，${what}；App 会把它按原样放回`,
      );
    } else {
      violation(
        "queueVerbatim",
        file.path,
        `${kind} ${file.path} 的正文没有逐字出现在 raw/ 里（应该写进 ${target}），它会留在收件箱`,
      );
    }
  });

  // Confirmed (✅) 底色 entries stay exactly as they are.
  const dise = changes.get(DISE_PATH);
  if (dise) {
    if (dise.opaque !== null) {
      violation("confirmedDise", DISE_PATH, `没法核对 ${DISE_PATH}（${OPAQUE_TEXT[dise.opaque]}）`);
    } else {
      const confirmed = (text: string | null) => (text === null ? [] : diseEntryTexts(text).filter((e) => e.confirmed));
      const old = confirmed(dise.before);
      const left = confirmed(dise.after);
      if (dise.status === "deleted" && old.length > 0) {
        violation("confirmedDise", DISE_PATH, `${DISE_PATH} 被删掉了，里面 ${old.length} 条已确认（✅）的条目也没了`);
      } else {
        for (const entry of old) {
          const index = left.findIndex((candidate) => candidate.id === entry.id);
          if (index === -1) {
            violation("confirmedDise", DISE_PATH, `已确认的 ${entry.id} 的 ✅ 不见了（${DISE_PATH}）`);
            continue;
          }
          const [match] = left.splice(index, 1);
          if (match && match.text !== entry.text) {
            violation("confirmedDise", DISE_PATH, `已确认（✅）的 ${entry.id} 被改动了（${DISE_PATH}），带 ✅ 的条目一个字都不能改`);
          }
        }
        for (const extra of left) {
          violation("confirmedDise", DISE_PATH, `${extra.id} 新加了 ✅（${DISE_PATH}），只有你能盖章`);
        }
      }
    }
  }

  // The 底色 and systems/ are the user's layer: other changes wait for them.
  if (dise && dise.opaque === null) approvals.push(...diseApprovals(dise.before, dise.after));
  for (const change of input.changes) {
    if (!change.path.startsWith("systems/") || queuePaths.has(change.path)) continue;
    const what = change.status === "added" ? "新建了" : change.status === "deleted" ? "删除了" : "改了";
    approvals.push({
      rule: "systems",
      path: change.path,
      entry: null,
      message: `${what}体系文件 ${change.path}：体系和机制以你为准，改动要你确认`,
      before: null,
      after: null,
    });
  }

  // The user's words in existing precedents stay exactly as they are.
  checkPrecedents(input.changes, violation);

  // No new broken wikilinks. A link copied verbatim from a queue file into
  // raw/ has to stay as it is: that one is only worth a warning.
  for (const ref of newBrokenLinks(before, after)) {
    const copied = ref.from.startsWith("raw/") && queueBodies.some((body) => body.includes(ref.link.raw));
    if (copied) {
      warning("brokenLinks", ref.from, `${ref.from} 里有断链 ${ref.link.raw}，它是原话里本来就有的，逐字保留`);
    } else {
      violation("brokenLinks", ref.from, `${ref.from} 里有新的断链 ${ref.link.raw}（第 ${ref.link.line + 1} 行附近）`);
    }
  }

  // 入库 writes a report; 落裁决 marks the answered questions.
  const reports = input.changes.filter(
    (change) =>
      change.path.startsWith("reports/") &&
      change.after !== null &&
      (change.status === "added" || (change.before !== null && change.after.length > change.before.length)),
  );
  if (ingest && reports.length === 0) {
    violation("report", null, `入库没有生成报告：reports/ 里没有新建或补充的入库报告（应该是 reports/${input.date}-入库报告.md）`);
  }
  if (!ingest) checkRulings(input, after, violation, warning);

  // The final JSON line: only ever a warning.
  const final = input.finalMessage === null ? null : parseFinalLine(input.finalMessage);
  if (final === null) {
    warning("finalLine", null, "AI 最后一条回复不是规定的那一行 JSON（不影响提交）");
  } else {
    const problems: string[] = [];
    const task = ingest ? "入库" : "落裁决";
    if (final["任务"] !== task) problems.push(`任务应该是"${task}"`);
    if (Number(final["条数"]) !== input.queue.length) {
      problems.push(`条数应该是 ${input.queue.length}`);
    }
    const named = final["报告"];
    if (ingest && typeof named === "string" && named.trim() !== "") {
      const path = named.trim().startsWith("reports/") ? named.trim() : `reports/${named.trim()}`;
      if (!reports.some((report) => report.path === path)) problems.push(`报告 ${named.trim()} 没有新建或补充`);
    }
    if (problems.length > 0) {
      warning("finalLine", null, `AI 最后那行 JSON 和实际不符：${problems.join("；")}（不影响提交）`);
    }
  }

  const commitPaths = input.changes
    .map((change) => change.path)
    .filter((path) => !queuePaths.has(path))
    .sort(byCodeUnit);
  const ok = violations.length === 0 && approvals.length === 0;
  const unit = ingest ? "条随手记" : "份裁决";
  const notes = warnings.length > 0 ? `（另有 ${warnings.length} 条提醒）` : "";
  const asks = `${approvals.length} 处改动需要你确认`;
  const touches = `${approvals.length} 处改动动到了你的底色或体系`;
  const summary = ok
    ? `通过检查：${deleteQueuePaths.length} ${unit}都逐字进了原料层，提交 ${commitPaths.length} 个文件${notes}`
    : violations.length > 0
      ? `没通过检查：${violations.length} 个问题${approvals.length > 0 ? `，另有 ${asks}` : ""}${notes}`
      : `需要你确认：${touches}，其余都通过了检查${notes}`;

  return {
    ok,
    violations,
    warnings,
    approvals,
    commitPaths,
    deleteQueuePaths,
    restoreQueuePaths,
    summary,
    final,
  };
}

const DISE_FILE = "self/底色.md";
const YOURS = "底色以你为主，改动要你确认";

/** The frontmatter, title and preface of the 底色; and its section headings with their notes. */
function diseFrame(text: string | null): { head: string; sections: string } {
  if (text === null) return { head: "", sections: "" };
  const doc = parseDise(text);
  const head = [splitFrontmatter(text).block.trimEnd(), doc.title === null ? "" : `# ${doc.title}`, doc.introMarkdown];
  const sections = doc.sections.flatMap((section) => [section.heading ? `## ${section.heading}` : "", section.introMarkdown]);
  const join = (parts: string[]) => parts.filter((part) => part !== "").join("\n\n");
  return { head: join(head), sections: join(sections) };
}

/**
 * What of a 底色 change needs the user's approval (AGENTS.md sections 三 and
 * 九): every existing entry that changed or went away, confirmed or not (a
 * confirmed one is a violation as well, reported elsewhere); every new entry
 * without the draft marker; and the frame around the entries. New draft
 * entries and blank lines between entries need nothing.
 */
function diseApprovals(before: string | null, after: string | null): GateApproval[] {
  const approvals: GateApproval[] = [];
  const old = before === null ? [] : diseEntryTexts(before);
  const now = after === null ? [] : diseEntryTexts(after);
  const left: DiseEntryText[] = [...now];
  const named = (entry: DiseEntryText) => `${entry.id}「${entry.title}」`;
  for (const entry of old) {
    const index = left.findIndex((candidate) => candidate.id === entry.id);
    const match = index === -1 ? undefined : left.splice(index, 1)[0];
    if (entry.confirmed) continue;
    if (!match) {
      approvals.push({
        rule: "diseEntry",
        path: DISE_FILE,
        entry: entry.id,
        message: `删掉了已有的底色条目 ${named(entry)}：${YOURS}`,
        before: entry.text,
        after: null,
      });
    } else if (match.text !== entry.text && !match.confirmed) {
      approvals.push({
        rule: "diseEntry",
        path: DISE_FILE,
        entry: entry.id,
        message: `改了已有的底色条目 ${named(entry)}：已有条目盖没盖章都一样，${YOURS}`,
        before: entry.text,
        after: match.text,
      });
    }
  }
  for (const entry of left) {
    if (entry.draft || entry.confirmed) continue;
    approvals.push({
      rule: "diseDraft",
      path: DISE_FILE,
      entry: entry.id,
      message: `新加的底色条目 ${named(entry)}没有标「${DRAFT_MARKER_TEXT}」：不是草稿的新条目要你确认`,
      before: null,
      after: entry.text,
    });
  }
  const was = diseFrame(before);
  const is = diseFrame(after);
  if (was.head !== is.head) {
    approvals.push({
      rule: "diseFrame",
      path: DISE_FILE,
      entry: null,
      message: `改了底色的开头（frontmatter、标题或前言）：${YOURS}`,
      before: was.head || null,
      after: is.head || null,
    });
  }
  if (was.sections !== is.sections) {
    approvals.push({
      rule: "diseFrame",
      path: DISE_FILE,
      entry: null,
      message: `改了底色的分节标题或分节说明：${YOURS}`,
      before: was.sections || null,
      after: is.sections || null,
    });
  }
  return approvals;
}

/** 落裁决: each answered Q file says 已裁决; a rejected audit item became a new Q file. */
function checkRulings(
  input: GateInput,
  after: Side,
  violation: (rule: GateRule, path: string | null, message: string) => void,
  warning: (rule: GateRule, path: string | null, message: string) => void,
) {
  const rulings: Ruling[] = [...after.text.entries()]
    .filter(([path]) => RULING_FILE.test(path))
    .map(([path, content]) => parseRuling(path, content));
  const added = input.changes
    .filter((change) => change.status === "added" && change.after !== null && RULING_FILE.test(change.path))
    .map((change) => parseRuling(change.path, change.after ?? ""));

  for (const file of input.queue) {
    const object = fmString(parseFrontmatter(file.content).data, "对象");
    const id = object ? /^Q-\d+/.exec(object)?.[0] : undefined;
    if (id) {
      const ruling = rulings.find((candidate) => candidate.id === id);
      if (!ruling) {
        violation("rulingsAnswered", file.path, `${file.path} 回答的是 ${id}，但 self/待裁决/ 里找不到 ${id}`);
      } else if (ruling.pending) {
        violation("rulingsAnswered", ruling.path, `${id} 的状态还不是"已裁决"（${ruling.path}）`);
      }
    } else if (object?.startsWith("抽查")) {
      if (added.length === 0) {
        violation("rulingsAnswered", file.path, `${file.path} 驳回了一条抽查，但没有新建待裁决文件`);
      } else if (!added.some((ruling) => (ruling.category ?? "").includes("抽查驳回"))) {
        warning("rulingsAnswered", file.path, `${file.path} 驳回了一条抽查，新建的待裁决文件分类不是"抽查驳回"`);
      }
    } else {
      warning("rulingsAnswered", file.path, `${file.path} 没写清对象（Q-xxx 或"抽查 …"），没法核对它落到了哪里`);
    }
  }
}

/**
 * Every precedent that existed before the run keeps the body of its `## 裁决`
 * sections byte for byte (AGENTS.md sections 三 and 九) and is neither deleted
 * nor renamed. New precedents may say anything.
 */
function checkPrecedents(
  changes: readonly GateFile[],
  violation: (rule: GateRule, path: string | null, message: string) => void,
) {
  const words = "这一节是你的原话，一个字都不能改";
  const added = changes.filter((change) => change.status === "added" && PRECEDENT_FILE.test(change.path));
  for (const change of changes) {
    if (change.status === "added" || !PRECEDENT_FILE.test(change.path)) continue;
    const path = change.path;
    if (change.status === "deleted") {
      const old = change.before === null ? null : precedentRulingTexts(change.before);
      const twin = old
        ? added.find((candidate) => candidate.after !== null && sameTexts(precedentRulingTexts(candidate.after), old))
        : undefined;
      violation(
        "precedentRuling",
        path,
        twin
          ? `把已有判例 ${path} 改名成了 ${twin.path}（判例里有你的原话，已有判例不能改名；要推翻它就新建一个判例）`
          : `删除了已有判例 ${path}（判例里有你的原话，已有判例不能删除，也不能改名）`,
      );
      continue;
    }
    if (change.opaque !== null || change.before === null || change.after === null) {
      const why = change.opaque ? OPAQUE_TEXT[change.opaque] : "内容读不出来";
      violation("precedentRuling", path, `没法核对已有判例 ${path} 的${RULING_SECTION}一节（${why}）`);
      continue;
    }
    const before = precedentRulingTexts(change.before);
    if (before.length === 0) continue;
    const after = precedentRulingTexts(change.after);
    if (sameTexts(before, after)) continue;
    const message =
      after.length < before.length
        ? `已有判例 ${path} 的${RULING_SECTION}一节不见了（${words}）`
        : after.length > before.length && sameTexts(before, after.slice(0, before.length))
          ? `已有判例 ${path} 多了一节${RULING_SECTION}（已有判例的裁决不能改；要推翻它就新建一个判例）`
          : `已有判例 ${path} 的${RULING_SECTION}一节被改动了（${words}）`;
    violation("precedentRuling", path, message);
  }
}

function sameTexts(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((text, index) => text === b[index]);
}
