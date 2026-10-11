import { fmString, stripMarkdown, type KbDoc } from "../../domain";
import { rawGroupLabel } from "../../app/links";

export interface RawGroup {
  key: string;
  label: string;
  docs: KbDoc[];
}

/** raw/ subfolders in display order; others follow alphabetically. */
const RAW_ORDER = ["inbox", "对话", "books", "articles"];

const ISO_DATE = /(\d{4}-\d{2}-\d{2})/;

/** Date of a source: from its file name, else from 抓取 / 时间 / 更新 in the frontmatter. */
export function docDate(doc: Pick<KbDoc, "name" | "frontmatter">): string | null {
  const fromName = /^(\d{4}-\d{2}-\d{2})/.exec(doc.name)?.[1];
  if (fromName) return fromName;
  for (const key of ["抓取", "时间", "更新"]) {
    const match = ISO_DATE.exec(fmString(doc.frontmatter, key) ?? "");
    if (match?.[1]) return match[1];
  }
  return null;
}

/** Groups raw files by subfolder; each group newest first. */
export function groupRaw(docs: readonly KbDoc[]): RawGroup[] {
  const groups = new Map<string, KbDoc[]>();
  for (const doc of docs) {
    const key = doc.subgroup ?? "";
    const list = groups.get(key);
    if (list) list.push(doc);
    else groups.set(key, [doc]);
  }
  const rank = (key: string) => {
    const index = RAW_ORDER.indexOf(key);
    if (index !== -1) return index;
    return key === "" ? RAW_ORDER.length + 1 : RAW_ORDER.length;
  };
  const newestFirst = (a: KbDoc, b: KbDoc) => {
    const da = docDate(a) ?? "";
    const db = docDate(b) ?? "";
    if (da !== db) return da < db ? 1 : -1;
    return a.name < b.name ? -1 : a.name > b.name ? 1 : 0;
  };
  return [...groups.entries()]
    .sort(([a], [b]) => rank(a) - rank(b) || (a < b ? -1 : a > b ? 1 : 0))
    .map(([key, list]) => ({
      key,
      label: rawGroupLabel(key || null),
      docs: [...list].sort(newestFirst),
    }));
}

/** First words of a capture (inbox/*.md), which has no title of its own. */
export function captureSummary(doc: Pick<KbDoc, "body">, length = 60): string {
  const text = stripMarkdown(doc.body);
  return text.length > length ? `${text.slice(0, length)}…` : text;
}
