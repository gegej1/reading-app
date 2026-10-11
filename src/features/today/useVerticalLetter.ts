import { useEffect, useLayoutEffect, useRef, useState } from "react";

/** Quotes longer than this are always set horizontally. */
export const VERTICAL_LIMIT = 96;

// Layout effects do not run on the server (tests render to static markup).
const useIsomorphicLayoutEffect = typeof window === "undefined" ? useEffect : useLayoutEffect;

export interface Edges {
  left: number;
  right: number;
}

/**
 * Width (border box) the vertical letter needs: from the leftmost to the
 * rightmost child, plus the paper's own padding and borders. Children may lie
 * partly outside the paper when they overflow.
 */
export function verticalExtent(children: readonly Edges[], frame: { start: number; end: number }): number {
  if (children.length === 0) return frame.start + frame.end;
  const left = Math.min(...children.map((child) => child.left));
  const right = Math.max(...children.map((child) => child.right));
  return Math.ceil(right - left + frame.start + frame.end);
}

function measureVertical(paper: HTMLElement): number {
  const style = getComputedStyle(paper);
  const px = (value: string) => Number.parseFloat(value) || 0;
  const children = [...paper.children].map((child) => {
    const rect = child.getBoundingClientRect();
    const margins = getComputedStyle(child);
    return { left: rect.left - px(margins.marginLeft), right: rect.right + px(margins.marginRight) };
  });
  return verticalExtent(children, {
    start: px(style.paddingLeft) + px(style.borderLeftWidth),
    end: px(style.paddingRight) + px(style.borderRightWidth),
  });
}

/**
 * Whether the letter is set vertically (竖排). Vertical columns cannot grow
 * downwards, so a long quote or a narrow window (an iPhone) pushes the
 * signature past the left edge of the paper, where WebKit's overlay
 * scrollbars give no hint that it could be scrolled into view. So a quote is
 * set vertically only when the whole letter, signature included, fits: the
 * width that layout needs is measured once per quote (it does not depend on
 * the window), and every resize compares it with the width available.
 */
export function useVerticalLetter(quote: string) {
  const paperRef = useRef<HTMLDivElement>(null);
  const [needed, setNeeded] = useState<{ quote: string; width: number } | null>(null);
  const [available, setAvailable] = useState<number | null>(null);

  const candidate = quote.length <= VERTICAL_LIMIT;
  const measured = needed?.quote === quote ? needed.width : null;
  const vertical = candidate && (measured === null || available === null || measured <= available);

  // Before the first paint of a new quote the letter is vertical: measure it
  // and switch to horizontal right away if it does not fit.
  useIsomorphicLayoutEffect(() => {
    const paper = paperRef.current;
    if (!paper || !candidate || measured !== null) return;
    setNeeded({ quote, width: measureVertical(paper) });
    setAvailable(paper.getBoundingClientRect().width);
  }, [quote, candidate, measured]);

  useEffect(() => {
    const paper = paperRef.current;
    if (!paper || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => setAvailable(paper.getBoundingClientRect().width));
    observer.observe(paper);
    return () => observer.disconnect();
  }, []);

  return { vertical, paperRef };
}
