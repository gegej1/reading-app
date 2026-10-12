// What the iPhone bridge may export, and nothing else.
//
// Only the 底色 layer leaves the Mac: self/底色.md, the precedents in
// self/判例/, the pending rulings in self/待裁决/ and the ingest reports in
// reports/ (of which only the 待你抽查 lines and, of the latest report, the
// sections about rulings are shown). raw/, wiki/, systems/, inbox/,
// index.md, log.md, the rest of self/ and anything else never reach the
// export: the files are filtered here before anything is parsed.

const DISE = "self/底色.md";
const IN_FOLDER = (folder: string) => new RegExp(`^${folder}/[^/]+\\.md$`);
const PRECEDENT = IN_FOLDER("self/判例");
const RULING = IN_FOLDER("self/待裁决");
const REPORT = IN_FOLDER("reports");

/** True for the files the export may read. */
export function isExportSource(path: string): boolean {
  return path === DISE || PRECEDENT.test(path) || RULING.test(path) || REPORT.test(path);
}

/** The pages of the export (the file names the Mac app accepts, see src-tauri/src/bridge/export.rs). */
export const PAGE_FILES = {
  today: "今日.html",
  dise: "底色.html",
  precedents: "判例.html",
  rulings: "待裁决.html",
  audits: "待抽查.html",
  report: "最近入库.html",
} as const;

export type PageKey = keyof typeof PAGE_FILES;

export const TEXT_FILES = {
  daily: "今日一条.txt",
  rulingList: "待裁决列表.txt",
} as const;

/** `待裁决/Q-001.txt`. */
export function questionFile(id: string): string {
  return `待裁决/${id}.txt`;
}
