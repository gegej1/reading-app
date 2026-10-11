import type { DocGroup, KbDoc, KbModel } from "../domain";
import type { LinkResolver } from "../ui/Markdown";

/** The five sections of the left navigation. */
export type NavSection = "today" | "dise" | "rulings" | "knowledge" | "raw";

export const NAV_ITEMS: { section: NavSection; label: string; to: string }[] = [
  { section: "today", label: "今日", to: "/" },
  { section: "dise", label: "底色", to: "/dise" },
  { section: "rulings", label: "裁决", to: "/rulings" },
  { section: "knowledge", label: "知识", to: "/knowledge" },
  { section: "raw", label: "原料", to: "/raw" },
];

/** Which navigation section a KB file belongs to. */
export function navSectionForGroup(group: DocGroup): NavSection {
  switch (group) {
    case "dise":
      return "dise";
    case "ruling":
    case "rulingIndex":
    case "report":
    case "answer":
      return "rulings";
    case "raw":
    case "capture":
      return "raw";
    default:
      return "knowledge";
  }
}

/** Route of the generic page view for a file basename. */
export function pageRoute(name: string): string {
  return `/page/${encodeURIComponent(name)}`;
}

/** Route that shows a document: 底色 has its own screen, everything else the page view. */
export function docRoute(doc: Pick<KbDoc, "name" | "group">): string {
  return doc.group === "dise" ? "/dise" : pageRoute(doc.name);
}

/** Route of one 底色 entry. */
export function diseEntryRoute(id: string): string {
  return `/dise/${encodeURIComponent(id)}`;
}

/** A plain `href` for a route under the HashRouter. */
export function hashHref(route: string): string {
  return `#${route}`;
}

/** Decodes a route parameter without throwing on stray `%`. */
export function safeDecode(value: string | undefined): string {
  if (value === undefined) return "";
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

/** Resolves wikilink targets to in-app links for the Markdown renderer. */
export function makeLinkResolver(model: Pick<KbModel, "links" | "byPath">): LinkResolver {
  return (target) => {
    const path = model.links.index.resolve(target);
    const doc = path ? model.byPath.get(path) : undefined;
    if (!path || !doc) return null;
    return { href: hashHref(docRoute(doc)), title: path };
  };
}

const RAW_LABELS: Record<string, string> = {
  inbox: "收件箱",
  对话: "对话",
  books: "书",
  articles: "文章",
};

/** Chinese label for a raw/ subfolder. */
export function rawGroupLabel(subgroup: string | null): string {
  if (!subgroup) return "原料";
  return RAW_LABELS[subgroup] ?? subgroup;
}

/** Short Chinese label of where a document lives. */
export function groupLabel(doc: Pick<KbDoc, "group" | "subgroup">): string {
  switch (doc.group) {
    case "dise":
      return "底色";
    case "precedent":
      return "判例";
    case "ruling":
      return "待裁决";
    case "rulingIndex":
      return "待裁决目录";
    case "self":
      return "底色层";
    case "report":
      return "入库报告";
    case "wiki":
      return doc.subgroup ?? "知识";
    case "system":
      return "体系";
    case "raw":
      return rawGroupLabel(doc.subgroup);
    case "capture":
      return "收件箱";
    case "answer":
      return "待落裁决";
    case "other":
      return "总览";
  }
}
