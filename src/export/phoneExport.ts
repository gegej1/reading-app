// The iPhone export: the 底色 layer as phone pages and short text files for
// the shortcuts. Pure: give it the knowledge-base files and the time, get the
// files to write into the bridge folder's 阅读/. Files outside the 底色 layer
// are dropped before anything is parsed (see ./sources).

import { buildKbModel, pickDailyEntry, stripMarkdown, type KbFile, type KbModel, type Ruling } from "../domain";
import { formatIsoDate } from "../ui/format";
import { renderAudits, renderDise, renderPrecedents, renderReport, renderRulings, renderToday } from "./phonePages";
import { isExportSource, PAGE_FILES, questionFile, TEXT_FILES } from "./sources";

export interface ExportFile {
  /** Relative to 阅读/. */
  name: string;
  content: string;
}

/** The model of the 底色 layer only. */
export function exportModel(files: readonly KbFile[]): KbModel {
  return buildKbModel(files.filter((file) => isExportSource(file.path)));
}

const MAX_SHORT = 400;

function short(text: string, limit = MAX_SHORT): string {
  const flat = stripMarkdown(text);
  return flat.length > limit ? `${flat.slice(0, limit)}…` : flat;
}

/** A ruling as the 答裁决 shortcut shows it before asking for the answer. */
export function questionText(ruling: Ruling): string {
  const suggestion = ruling.sections.find((s) => s.heading.startsWith("AI 建议"));
  const lines = [`${ruling.id} ${ruling.title}`];
  const meta = [ruling.priority ? `优先级 ${ruling.priority}` : null, ruling.category].filter(Boolean).join(" · ");
  if (meta) lines.push(meta);
  lines.push("", `要你定：${ruling.question ? short(ruling.question) : "（问题页里没有写「要你定」）"}`);
  if (suggestion && suggestion.markdown.trim() !== "") {
    lines.push("", `AI 建议（参考）：${short(suggestion.markdown, 300)}`);
  }
  return `${lines.join("\n")}\n`;
}

/** `Q-001｜标题`, one per open ruling: the list 答裁决 chooses from. */
export function rulingListText(model: KbModel): string {
  return answerable(model)
    .map((r) => `${r.id}｜${r.title.replace(/[\r\n]+/g, " ")}\n`)
    .join("");
}

/** Open rulings the phone can answer: a `Q-001` id, each id once. */
function answerable(model: KbModel): Ruling[] {
  const seen = new Set<string>();
  return model.pendingRulings.filter((r) => {
    if (!/^Q-\d+$/.test(r.id) || seen.has(r.id)) return false;
    seen.add(r.id);
    return true;
  });
}

/** Today's principle as plain text. */
export function dailyText(model: KbModel, now: Date): string {
  const entry = pickDailyEntry(model.dise?.entries ?? [], now);
  const date = formatIsoDate(
    `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`,
  );
  if (!entry) return `${date}\n\n底色里还没有条目。\n`;
  const mark = entry.confirmed ? " ✅" : entry.draft ? "（草稿，待你确认）" : "";
  const quote = entry.quote ? `\n\n${stripMarkdown(entry.quote)}` : "";
  return `${entry.id} ${entry.title}${mark}${quote}\n\n— ${date} · 今日一条\n`;
}

/** Every file of the export, in a stable order. */
export function buildPhoneExport(files: readonly KbFile[], now: Date): ExportFile[] {
  const model = exportModel(files);
  const out: ExportFile[] = [
    { name: PAGE_FILES.today, content: renderToday(model, now) },
    { name: PAGE_FILES.dise, content: renderDise(model) },
    { name: PAGE_FILES.precedents, content: renderPrecedents(model) },
    { name: PAGE_FILES.rulings, content: renderRulings(model) },
    { name: PAGE_FILES.audits, content: renderAudits(model) },
    { name: PAGE_FILES.report, content: renderReport(model) },
    { name: TEXT_FILES.daily, content: dailyText(model, now) },
    { name: TEXT_FILES.rulingList, content: rulingListText(model) },
  ];
  for (const ruling of answerable(model)) {
    out.push({ name: questionFile(ruling.id), content: questionText(ruling) });
  }
  return out;
}
