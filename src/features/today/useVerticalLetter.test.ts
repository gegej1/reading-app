import { describe, expect, it } from "vitest";
import { verticalExtent } from "./useVerticalLetter";

describe("verticalExtent", () => {
  const frame = { start: 47, end: 47 };

  it("spans every child plus the paper's padding and borders", () => {
    // Quote columns on the right, the signature column after a gap on the left.
    expect(verticalExtent([{ left: 230, right: 506 }, { left: 138, right: 184 }], frame)).toBe(462);
  });

  it("counts children that overflow past the left edge", () => {
    expect(verticalExtent([{ left: -60, right: 300 }, { left: -150, right: -104 }], frame)).toBe(544);
  });

  it("rounds up and handles an empty paper", () => {
    expect(verticalExtent([{ left: 0.2, right: 100.1 }], { start: 0, end: 0 })).toBe(100);
    expect(verticalExtent([{ left: 0.2, right: 100.3 }], { start: 0, end: 0 })).toBe(101);
    expect(verticalExtent([], frame)).toBe(94);
  });
});
