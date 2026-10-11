// Chinese date formatting for the UI.

const DIGITS = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
const WEEKDAYS = ["星期日", "星期一", "星期二", "星期三", "星期四", "星期五", "星期六"];

/** 1–99 in Chinese numerals as used in dates: 12 -> 十二, 20 -> 二十. */
export function chineseNumber(n: number): string {
  if (n < 10) return DIGITS[n] ?? String(n);
  const tens = Math.floor(n / 10);
  const ones = n % 10;
  return `${tens === 1 ? "" : (DIGITS[tens] ?? "")}十${ones === 0 ? "" : (DIGITS[ones] ?? "")}`;
}

/** Parts of the 今日 header for a local date. */
export function todayLabel(date: Date): { monthDay: string; year: string; weekday: string } {
  return {
    monthDay: `${chineseNumber(date.getMonth() + 1)}月${chineseNumber(date.getDate())}日`,
    year: String(date.getFullYear()),
    weekday: WEEKDAYS[date.getDay()] ?? "",
  };
}

/** `2026-10-11` -> `2026 年 10 月 11 日`; other text is returned unchanged. */
export function formatIsoDate(
  iso: string | null | undefined,
  options: { withYear?: boolean } = {},
): string {
  if (!iso) return "";
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso.trim());
  if (!match) return iso;
  const month = Number(match[2]);
  const day = Number(match[3]);
  const short = `${month} 月 ${day} 日`;
  return options.withYear === false ? short : `${match[1]} 年 ${short}`;
}
