import { describe, expect, it } from "vitest";
import { APP_DISPLAY_NAME, formatVersionLabel } from "./version";

describe("APP_DISPLAY_NAME", () => {
  it("is the product name shown to the user", () => {
    expect(APP_DISPLAY_NAME).toBe("底色");
  });
});

describe("formatVersionLabel", () => {
  it("appends the version to the display name", () => {
    expect(formatVersionLabel("0.1.0")).toBe("底色 v0.1.0");
  });

  it("tolerates a leading v and surrounding whitespace", () => {
    expect(formatVersionLabel(" v1.2.3 ")).toBe("底色 v1.2.3");
    expect(formatVersionLabel("V2.0.0")).toBe("底色 v2.0.0");
  });

  it("falls back to the bare display name when the version is empty", () => {
    expect(formatVersionLabel("")).toBe("底色");
    expect(formatVersionLabel("   ")).toBe("底色");
  });
});
