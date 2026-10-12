import { indexAnswers, type AnswerIndex } from "./answers";
import { parseDise, type DiseDoc } from "./dise";
import { parseFrontmatter, type FrontmatterData } from "./frontmatter";
import { firstHeading } from "./markdown";
import { parsePrecedent, type Precedent } from "./precedents";
import {
  parseReport,
  sortReportsNewestFirst,
  type AuditItem,
  type Report,
} from "./reports";
import { parseRuling, sortRulings, type Ruling } from "./rulings";
import {
  analyzeLinks,
  basenameOf,
  buildLinkIndex,
  type LinkAnalysis,
  type LinkIndex,
} from "./wikilinks";

/** Where a file lives in the KB layout (see the KB's AGENTS.md). */
export type DocGroup =
  | "dise" // self/底色.md
  | "precedent" // self/判例/
  | "ruling" // self/待裁决/
  | "rulingIndex" // self/待裁决.md
  | "self" // other files under self/
  | "report" // reports/
  | "wiki" // wiki/
  | "system" // systems/
  | "raw" // raw/
  | "capture" // inbox/*.md
  | "answer" // inbox/裁决/
  | "other";

export interface KbDoc {
  path: string;
  /** Basename without `.md`: the wikilink name. */
  name: string;
  group: DocGroup;
  /** First folder below the group root (wiki/概念 -> 概念, raw/books -> books). */
  subgroup: string | null;
  title: string;
  frontmatter: FrontmatterData;
  body: string;
  content: string;
  mtime?: number;
}

export interface KbFile {
  path: string;
  content: string;
  mtime?: number;
}

export interface DocGroupList {
  name: string;
  docs: KbDoc[];
}

export interface LatestReportSummary {
  path: string;
  name: string;
  date: string | null;
  title: string | null;
  resultLine: string | null;
  resultParts: string[];
}

export interface DashboardCounts {
  /** Capture files waiting in inbox/ (not inbox/裁决/). */
  inboxCaptures: number;
  /** Answered rulings waiting in inbox/裁决/ for 落裁决. */
  answeredRulings: number;
  pendingRulings: { total: number; high: number; medium: number; low: number; other: number };
  /** Unchecked 待你抽查 items across all reports. */
  uncheckedAudit: number;
  latestReport: LatestReportSummary | null;
}

export interface KbModel {
  docs: KbDoc[];
  byPath: Map<string, KbDoc>;
  /** Basename -> document (first path in sort order when names collide). */
  byName: Map<string, KbDoc>;
  dise: DiseDoc | null;
  disePath: string | null;
  precedents: Precedent[];
  /** All rulings, sorted by priority then id. */
  rulings: Ruling[];
  pendingRulings: Ruling[];
  /** Newest first. */
  reports: Report[];
  /** Audit items of all reports, newest report first. */
  auditItems: AuditItem[];
  wikiGroups: DocGroupList[];
  systems: KbDoc[];
  raw: KbDoc[];
  captures: KbDoc[];
  answers: KbDoc[];
  /** The answers in inbox/裁决/ by what they answer (rulings, rejected audit items). */
  answerIndex: AnswerIndex;
  links: LinkAnalysis & { index: LinkIndex };
  dashboard: DashboardCounts;
}

export const DISE_PATH = "self/底色.md";

/** Known wiki folders first, in this order; others follow alphabetically. */
const WIKI_ORDER = ["概念", "人物与公司", "书"];

const byPath = (a: { path: string }, b: { path: string }) =>
  a.path < b.path ? -1 : a.path > b.path ? 1 : 0;
const byName = (a: { name: string }, b: { name: string }) =>
  a.name < b.name ? -1 : a.name > b.name ? 1 : 0;

/** Maps a repo-relative path to its group in the KB layout. */
export function classifyPath(path: string): { group: DocGroup; subgroup: string | null } {
  const parts = path.split("/");
  const top = parts[0];
  const depth = parts.length;
  const sub = depth > 2 ? (parts[1] ?? null) : null;

  if (path === DISE_PATH) return { group: "dise", subgroup: null };
  if (path === "self/待裁决.md") return { group: "rulingIndex", subgroup: null };
  if (top === "self") {
    if (parts[1] === "判例" && depth > 2) return { group: "precedent", subgroup: null };
    if (parts[1] === "待裁决" && depth > 2) return { group: "ruling", subgroup: null };
    return { group: "self", subgroup: null };
  }
  if (top === "reports") return { group: "report", subgroup: null };
  if (top === "wiki") return { group: "wiki", subgroup: sub };
  if (top === "systems") return { group: "system", subgroup: sub };
  if (top === "raw") return { group: "raw", subgroup: sub };
  if (top === "inbox") {
    if (depth === 2) return { group: "capture", subgroup: null };
    if (parts[1] === "裁决" && depth === 3) return { group: "answer", subgroup: null };
  }
  return { group: "other", subgroup: null };
}

/** Title of a page: its first H1, else its file name. */
export function docTitle(path: string, body: string): string {
  return firstHeading(body, 1) ?? basenameOf(path);
}

function toDoc(file: KbFile): KbDoc {
  const { data, body } = parseFrontmatter(file.content);
  const { group, subgroup } = classifyPath(file.path);
  const doc: KbDoc = {
    path: file.path,
    name: basenameOf(file.path),
    group,
    subgroup,
    title: docTitle(file.path, body),
    frontmatter: data,
    body,
    content: file.content,
  };
  if (file.mtime !== undefined) doc.mtime = file.mtime;
  return doc;
}

function groupWiki(docs: KbDoc[]): DocGroupList[] {
  const groups = new Map<string, KbDoc[]>();
  for (const doc of docs) {
    const key = doc.subgroup ?? "其他";
    const list = groups.get(key);
    if (list) list.push(doc);
    else groups.set(key, [doc]);
  }
  const rank = (name: string) => {
    const index = WIKI_ORDER.indexOf(name);
    return index === -1 ? WIKI_ORDER.length : index;
  };
  return [...groups.entries()]
    .sort(([a], [b]) => rank(a) - rank(b) || (a < b ? -1 : a > b ? 1 : 0))
    .map(([name, list]) => ({ name, docs: [...list].sort(byName) }));
}

/** Builds the whole read model from file contents. Pure and synchronous. */
export function buildKbModel(files: readonly KbFile[]): KbModel {
  const docs = files.filter((f) => f.path.endsWith(".md")).map(toDoc).sort(byPath);
  const docsByPath = new Map(docs.map((doc) => [doc.path, doc]));
  const docsByName = new Map<string, KbDoc>();
  for (const doc of docs) if (!docsByName.has(doc.name)) docsByName.set(doc.name, doc);
  const inGroup = (group: DocGroup) => docs.filter((doc) => doc.group === group);

  const diseDoc = docsByPath.get(DISE_PATH) ?? null;
  const precedents = inGroup("precedent")
    .map((doc) => parsePrecedent(doc.path, doc.content))
    .sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  const rulings = sortRulings(inGroup("ruling").map((doc) => parseRuling(doc.path, doc.content)));
  const pendingRulings = rulings.filter((r) => r.pending);
  const reports = sortReportsNewestFirst(
    inGroup("report").map((doc) => parseReport(doc.path, doc.content)),
  );
  const auditItems = reports.flatMap((r) => r.auditItems);
  const captures = inGroup("capture");
  const answers = inGroup("answer");

  const index = buildLinkIndex(docs.map((doc) => doc.path));
  const links = { index, ...analyzeLinks(docs, index) };

  const count = (priority: string) =>
    pendingRulings.filter((r) => r.priority === priority).length;
  const high = count("高");
  const medium = count("中");
  const low = count("低");
  const latest = reports[0];

  return {
    docs,
    byPath: docsByPath,
    byName: docsByName,
    dise: diseDoc ? parseDise(diseDoc.content) : null,
    disePath: diseDoc ? diseDoc.path : null,
    precedents,
    rulings,
    pendingRulings,
    reports,
    auditItems,
    wikiGroups: groupWiki(inGroup("wiki")),
    systems: inGroup("system"),
    raw: inGroup("raw"),
    captures,
    answers,
    answerIndex: indexAnswers(answers),
    links,
    dashboard: {
      inboxCaptures: captures.length,
      answeredRulings: answers.length,
      pendingRulings: {
        total: pendingRulings.length,
        high,
        medium,
        low,
        other: pendingRulings.length - high - medium - low,
      },
      uncheckedAudit: auditItems.filter((item) => !item.checked).length,
      latestReport: latest
        ? {
            path: latest.path,
            name: latest.name,
            date: latest.date,
            title: latest.title,
            resultLine: latest.resultLine,
            resultParts: latest.resultParts,
          }
        : null,
    },
  };
}

/** Runs `task` over `items` with at most `limit` promises in flight. */
async function mapLimit<T, R>(
  items: readonly T[],
  limit: number,
  task: (item: T) => Promise<R>,
): Promise<R[]> {
  const results = new Array<R>(items.length);
  let next = 0;
  const worker = async () => {
    while (next < items.length) {
      const index = next;
      next += 1;
      results[index] = await task(items[index] as T);
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, worker));
  return results;
}

/**
 * What loadKb needs from a store. Declared here so the domain layer does not
 * depend on the store layer; every KbStore satisfies it.
 */
export interface KbSource {
  list(): Promise<{ path: string; mtime?: number | null }[]>;
  read(path: string): Promise<string>;
}

/** Reads every Markdown file through the store and builds the model. */
export async function loadKb(
  store: KbSource,
  options: { concurrency?: number } = {},
): Promise<KbModel> {
  const entries = (await store.list()).filter((entry) => entry.path.endsWith(".md"));
  const files = await mapLimit(entries, options.concurrency ?? 8, async (entry) => {
    try {
      const content = await store.read(entry.path);
      const file: KbFile = { path: entry.path, content };
      if (entry.mtime !== undefined && entry.mtime !== null) file.mtime = entry.mtime;
      return file;
    } catch (error) {
      const reason = error instanceof Error ? error.message : String(error);
      throw new Error(`读取 ${entry.path} 失败：${reason}`);
    }
  });
  return buildKbModel(files);
}
