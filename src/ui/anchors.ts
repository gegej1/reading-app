// In-page anchors for rendered KB Markdown: stable heading ids, and finding
// the heading a `[文字](#小节)` link points at. The app runs under a hash
// router, so such links must scroll instead of changing `location.hash`.

/** Letters (any script), marks, digits, whitespace, `_` and `-` survive; the rest is dropped. */
const DROPPED = /[^\p{L}\p{M}\p{N}\s_-]/gu;
/** Emoji variation selectors are marks too, but invisible: drop them first. */
const VARIATION_SELECTORS = new RegExp("[\\uFE00-\\uFE0F]", "g");

function decode(fragment: string): string {
  try {
    return decodeURIComponent(fragment);
  } catch {
    return fragment;
  }
}

/**
 * GitHub-style slug: lower case, punctuation and symbols dropped (、 ✅ ！),
 * whitespace runs turned into one hyphen. CJK text is kept as it is.
 */
export function headingSlug(text: string): string {
  const slug = text
    .toLowerCase()
    .replace(VARIATION_SELECTORS, "")
    .replace(DROPPED, "")
    .trim()
    .replace(/\s+/g, "-");
  return slug === "" ? "section" : slug;
}

/** Returns unique slugs in call order: 小节, 小节-1, 小节-2 … */
export function createSlugger(): (text: string) => string {
  const used = new Set<string>();
  return (text) => {
    const base = headingSlug(text);
    let slug = base;
    for (let n = 1; used.has(slug); n += 1) slug = `${base}-${n}`;
    used.add(slug);
    return slug;
  };
}

export interface AnchorCandidate {
  id: string;
  textContent: string | null;
}

/**
 * The heading an in-page link points at. `fragment` is the part after `#`
 * (possibly percent-encoded); it may be the id, any text that slugs to it, or
 * the heading text itself.
 */
export function findAnchorTarget<T extends AnchorCandidate>(
  candidates: Iterable<T>,
  fragment: string,
): T | null {
  const wanted = decode(fragment).trim();
  if (wanted === "") return null;
  const slug = headingSlug(wanted);
  const list = [...candidates];
  return (
    list.find((c) => c.id === wanted) ??
    list.find((c) => c.id === slug) ??
    list.find((c) => (c.textContent ?? "").trim() === wanted) ??
    list.find((c) => headingSlug(c.textContent ?? "") === slug) ??
    null
  );
}
