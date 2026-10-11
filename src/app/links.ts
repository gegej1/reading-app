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

const PAGE_PREFIX = "/page/";

/**
 * Route of the generic page view for one KB file, by its repo-relative path:
 * `wiki/概念/小实验.md` -> `/page/wiki/%E6%A6%82%E5%BF%B5/...`. Each segment is
 * encoded on its own, so `#`, `?` and `%` in file names survive the hash router.
 */
export function pageRoute(path: string): string {
  return `${PAGE_PREFIX}${path.split("/").map(encodeURIComponent).join("/")}`;
}

/** Route that shows a document: 底色 has its own screen, everything else the page view. */
export function docRoute(doc: Pick<KbDoc, "path" | "group">): string {
  return doc.group === "dise" ? "/dise" : pageRoute(doc.path);
}

/** The repo path in a page route's (still encoded) pathname, or null for other routes. */
export function pathFromPageRoute(pathname: string): string | null {
  if (!pathname.startsWith(PAGE_PREFIX)) return null;
  return pathname.slice(PAGE_PREFIX.length).split("/").map(safeDecode).join("/");
}

/**
 * The document a page route asks for. Routes carry the repo path; a bare name
 * (an old `/page/<name>` URL) is resolved like a wikilink.
 */
export function findPage(model: Pick<KbModel, "byPath" | "links">, requested: string): KbDoc | null {
  if (requested === "") return null;
  const exact = model.byPath.get(requested);
  if (exact) return exact;
  if (requested.includes("/")) return null;
  const path = model.links.index.resolve(requested);
  return path ? (model.byPath.get(path) ?? null) : null;
}

/** Other files with the same basename (the wikilink name) as `doc`, sorted. */
export function sameNamePaths(model: Pick<KbModel, "links">, doc: Pick<KbDoc, "path" | "name">): string[] {
  return (model.links.index.byName.get(doc.name) ?? []).filter((path) => path !== doc.path);
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

/**
 * Resolves wikilink targets to in-app links for the Markdown renderer.
 * Path-qualified targets (`[[wiki/书/README]]`) open exactly that file; a bare
 * name shared by several files opens the first path in code-point order (the
 * link index's rule), and the tooltip says the name is ambiguous.
 */
export function makeLinkResolver(model: Pick<KbModel, "links" | "byPath">): LinkResolver {
  return (target) => {
    const path = model.links.index.resolve(target);
    const doc = path ? model.byPath.get(path) : undefined;
    if (!path || !doc) return null;
    const namesakes = model.links.index.byName.get(doc.name)?.length ?? 1;
    const ambiguous = namesakes > 1 && !target.includes("/");
    return {
      href: hashHref(docRoute(doc)),
      title: ambiguous ? `${path}（同名文件共 ${namesakes} 个）` : path,
    };
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
