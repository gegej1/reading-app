import { fmString, parseFrontmatter, type FrontmatterData } from "./frontmatter";
import { docSections, identify, type DocSection } from "./precedents";
import { basenameOf } from "./wikilinks";

/** A pending ruling `self/待裁决/Q-001-标题.md`. */
export interface Ruling {
  path: string;
  name: string;
  id: string;
  title: string;
  shortTitle: string;
  /** `待裁决` or `已裁决` (free text). */
  status: string | null;
  /** True until the status says 已裁决. */
  pending: boolean;
  /** 新冲突 / 衍生问题 / 遗留 / 抽查驳回. */
  category: string | null;
  /** 高 / 中 / 低. */
  priority: string | null;
  source: string | null;
  updated: string | null;
  /** First paragraph of the 要你定 section: what the user has to decide. */
  question: string | null;
  frontmatter: FrontmatterData;
  sections: DocSection[];
  body: string;
}

const PRIORITY_RANK: Record<string, number> = { 高: 0, 中: 1, 低: 2 };

/** A ruling stays open until its status says it was decided. */
export function isPendingStatus(status: string | null): boolean {
  return status === null || !status.includes("已裁决");
}

/** Parses one ruling file. */
export function parseRuling(path: string, markdown: string): Ruling {
  const { data, body } = parseFrontmatter(markdown);
  const { id, title, shortTitle } = identify(path, fmString(data, "编号"), body, /^Q-\d+/);
  const sections = docSections(body);
  const status = fmString(data, "状态");
  const ask = sections.find((s) => s.heading.startsWith("要你定"));
  return {
    path,
    name: basenameOf(path),
    id,
    title,
    shortTitle,
    status,
    pending: isPendingStatus(status),
    category: fmString(data, "分类"),
    priority: fmString(data, "优先级"),
    source: fmString(data, "来源"),
    updated: fmString(data, "更新"),
    question: ask ? firstParagraph(ask.markdown) : null,
    frontmatter: data,
    sections,
    body,
  };
}

function firstParagraph(markdown: string): string | null {
  const paragraph = markdown.split(/\n\s*\n/)[0]?.trim() ?? "";
  return paragraph === "" ? null : paragraph.replace(/\s*\n\s*/g, " ");
}

/** Compares ids like `Q-002` and `Q-010` by their numeric part. */
export function compareIds(a: string, b: string): number {
  const na = Number(/\d+/.exec(a)?.[0] ?? Number.NaN);
  const nb = Number(/\d+/.exec(b)?.[0] ?? Number.NaN);
  if (!Number.isNaN(na) && !Number.isNaN(nb) && na !== nb) return na - nb;
  return a < b ? -1 : a > b ? 1 : 0;
}

/** Priority order 高, 中, 低, then anything else; ties broken by id. */
export function sortRulings(rulings: readonly Ruling[]): Ruling[] {
  const rank = (r: Ruling) => PRIORITY_RANK[r.priority ?? ""] ?? 3;
  return [...rulings].sort((a, b) => rank(a) - rank(b) || compareIds(a.id, b.id));
}
