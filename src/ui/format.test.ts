import { describe, expect, it } from "vitest";
import { chineseNumber, formatIsoDate, todayLabel } from "./format";

describe("chineseNumber", () => {
  it("writes 1–99 the way dates are written", () => {
    expect(chineseNumber(1)).toBe("一");
    expect(chineseNumber(10)).toBe("十");
    expect(chineseNumber(12)).toBe("十二");
    expect(chineseNumber(20)).toBe("二十");
    expect(chineseNumber(31)).toBe("三十一");
  });
});

describe("todayLabel", () => {
  it("formats a local date for the 今日 header", () => {
    expect(todayLabel(new Date(2026, 9, 12))).toEqual({
      monthDay: "十月十二日",
      year: "2026",
      weekday: "星期一",
    });
    expect(todayLabel(new Date(2026, 0, 4)).weekday).toBe("星期日");
  });
});

describe("formatIsoDate", () => {
  it("turns YYYY-MM-DD into a short Chinese date", () => {
    expect(formatIsoDate("2026-10-11")).toBe("2026 年 10 月 11 日");
    expect(formatIsoDate("2026-10-11", { withYear: false })).toBe("10 月 11 日");
    expect(formatIsoDate(null)).toBe("");
    expect(formatIsoDate("不是日期")).toBe("不是日期");
  });
});
