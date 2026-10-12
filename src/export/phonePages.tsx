// The exported phone pages, rendered to static HTML: self-contained (inline
// CSS, no scripts, a CSP that forbids everything else), phone width first,
// light and dark. Links between exported pages are relative; wikilinks to
// pages that are not exported are plain text.

import type { ComponentProps, ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  fmString,
  pickDailyEntry,
  splitSections,
  type DiseEntry,
  type KbModel,
  type Precedent,
  type Report,
  type Ruling,
} from "../domain";
import { chineseNumber, formatIsoDate, todayLabel } from "../ui/format";
import { remarkWikilinks, type WikilinkTarget } from "../ui/remarkWikilinks";
import { PHONE_CSS } from "./phoneStyles";
import { PAGE_FILES, type PageKey } from "./sources";

const CSP =
  "default-src 'none'; style-src 'unsafe-inline'; img-src data:; base-uri 'none'; form-action 'none'";

const NAV: { key: PageKey; label: string }[] = [
  { key: "today", label: "今日" },
  { key: "dise", label: "底色" },
  { key: "rulings", label: "待裁决" },
  { key: "audits", label: "待抽查" },
  { key: "precedents", label: "判例" },
  { key: "report", label: "最近入库" },
];

/** An id usable as an anchor: `D05`, `P-001`, `Q-001`. */
export function anchorId(id: string): string {
  return id.replace(/[^0-9A-Za-z_-]+/g, "-");
}

type Resolve = (target: string) => WikilinkTarget | null;

/** Wikilinks to exported pages become relative links; everything else is plain text. */
export function exportLinkResolver(model: KbModel): Resolve {
  return (target) => {
    const path = model.links.index.resolve(target);
    const doc = path ? model.byPath.get(path) : undefined;
    if (!path || !doc) return null;
    switch (doc.group) {
      case "dise":
        return { href: PAGE_FILES.dise };
      case "precedent": {
        const precedent = model.precedents.find((p) => p.path === path);
        return precedent ? { href: `${PAGE_FILES.precedents}#${anchorId(precedent.id)}` } : null;
      }
      case "ruling": {
        const ruling = model.pendingRulings.find((r) => r.path === path);
        return ruling ? { href: `${PAGE_FILES.rulings}#${anchorId(ruling.id)}` } : null;
      }
      case "report":
        return model.reports[0]?.path === path ? { href: PAGE_FILES.report } : null;
      default:
        return null;
    }
  };
}

const EXTERNAL = /^https?:\/\//i;
const EXPORTED = new RegExp(`^(${Object.values(PAGE_FILES).map((f) => f.replace(".", "\\.")).join("|")})(#[0-9A-Za-z_-]*)?$`);

const markdownComponents: Components = {
  a: ({ href, children }) =>
    href && (EXTERNAL.test(href) || EXPORTED.test(href)) ? <a href={href}>{children}</a> : <span className="plain">{children}</span>,
  span: ({ className, children }) => <span className={String(className ?? "").includes("wikilink") ? "plain" : undefined}>{children}</span>,
  img: ({ alt }) => (alt ? <span className="plain">{alt}</span> : null),
  input: ({ type, checked }) =>
    type === "checkbox" ? <input type="checkbox" checked={Boolean(checked)} disabled readOnly /> : null,
};

function Md({ children, resolve }: { children: string; resolve: Resolve }) {
  if (children.trim() === "") return null;
  const plugins = [remarkGfm, [remarkWikilinks, { resolve }]] as NonNullable<
    ComponentProps<typeof ReactMarkdown>["remarkPlugins"]
  >;
  return (
    <div className="prose">
      <ReactMarkdown remarkPlugins={plugins} components={markdownComponents}>
        {children}
      </ReactMarkdown>
    </div>
  );
}

/** The `## ` sections of a page as Markdown with their headings. */
function sectionsMarkdown(sections: readonly { heading: string; markdown: string }[]): string {
  return sections.map((s) => `## ${s.heading}\n\n${s.markdown}`).join("\n\n");
}

function Shell({ page, title, children }: { page: PageKey; title: string; children: ReactNode }) {
  return (
    <html lang="zh-CN">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />
        <meta name="color-scheme" content="light dark" />
        <meta httpEquiv="Content-Security-Policy" content={CSP} />
        <title>{`底色 · ${title}`}</title>
        <style dangerouslySetInnerHTML={{ __html: PHONE_CSS }} />
      </head>
      <body>
        <div className="wrap">
          <header className="mast">
            <div className="mast__row">
              <a className="seal" href={PAGE_FILES.today}>
                底色
              </a>
              <span className="mast__title">决策底色 · 手机版</span>
            </div>
            <nav className="mast__nav">
              {NAV.map((item) => (
                <a key={item.key} href={PAGE_FILES[item.key]} aria-current={item.key === page ? "page" : undefined}>
                  {item.label}
                </a>
              ))}
            </nav>
          </header>
          <main>{children}</main>
          <footer className="foot">从 Mac 上的「底色」导出 · 只读 · 要改请在 Mac 上</footer>
        </div>
      </body>
    </html>
  );
}

function Head({ eyebrow, title, meta, children }: { eyebrow: string; title: string; meta?: ReactNode; children?: ReactNode }) {
  return (
    <header className="head">
      <p className="eyebrow">{eyebrow}</p>
      <h1 className="title">{title}</h1>
      {meta ? <p className="meta">{meta}</p> : null}
      {children}
    </header>
  );
}

function render(page: PageKey, title: string, body: ReactNode): string {
  return `<!doctype html>\n${renderToStaticMarkup(
    <Shell page={page} title={title}>
      {body}
    </Shell>,
  )}\n`;
}

function EntryCard({ entry, resolve, linkTitle = false }: { entry: DiseEntry; resolve: Resolve; linkTitle?: boolean }) {
  const state = entry.confirmed ? "is-confirmed" : entry.draft ? "is-draft" : "";
  return (
    <article className={`card ${state}`.trim()} id={anchorId(entry.id)}>
      <div className="card__head">
        <span className="card__id num">{entry.id}</span>
        <h3 className="card__title">
          {linkTitle ? <a href={`${PAGE_FILES.dise}#${anchorId(entry.id)}`}>{entry.title}</a> : entry.title}
        </h3>
        <span className="card__mark">
          {entry.confirmed ? <span className="stamp">已确认</span> : entry.draft ? <span className="badge">草稿，待你确认</span> : null}
        </span>
      </div>
      {entry.notes.length > 0 ? (
        <div className="chips">
          {entry.notes.map((note) => (
            <span key={note} className="chip">
              {note}
            </span>
          ))}
        </div>
      ) : null}
      {entry.quote ? (
        <div className="quote">
          <Md resolve={resolve}>{entry.quote}</Md>
        </div>
      ) : null}
      {entry.bodyMarkdown ? (
        <div className="small">
          <Md resolve={resolve}>{entry.bodyMarkdown}</Md>
        </div>
      ) : null}
    </article>
  );
}

function rulingChips(ruling: Ruling) {
  return [
    ruling.priority ? { text: `优先级 ${ruling.priority}`, high: ruling.priority === "高" } : null,
    ruling.category ? { text: ruling.category } : null,
    ruling.updated ? { text: `更新 ${ruling.updated}` } : null,
  ].filter((chip): chip is { text: string; high?: boolean } => chip !== null);
}

/** Today: the date, one principle, the counts, the pages. */
export function renderToday(model: KbModel, now: Date): string {
  const resolve = exportLinkResolver(model);
  const label = todayLabel(now);
  const entries = model.dise?.entries ?? [];
  const entry = pickDailyEntry(entries, now);
  const pending = model.pendingRulings;
  const high = pending.filter((r) => r.priority === "高").length;
  const audits = model.auditItems.filter((item) => !item.checked).length;
  const confirmed = entries.filter((e) => e.confirmed).length;
  const drafts = entries.filter((e) => e.draft).length;
  const latest = model.reports[0];
  const time = `${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;
  return render(
    "today",
    "今日",
    <>
      <header className="head">
        <p className="eyebrow">
          今日 · {label.weekday}
        </p>
        <h1 className="today__date">{label.monthDay}</h1>
        <p className="meta">
          <span className="num">{label.year}</span> 年 · 导出于 <span className="num">{time}</span>
        </p>
      </header>
      <p className="eyebrow">今日一条</p>
      {entry ? (
        <div className="today__entry">
          <EntryCard entry={entry} resolve={resolve} linkTitle />
        </div>
      ) : (
        <p className="empty">底色里还没有条目</p>
      )}
      <div className="counts">
        <a className="count" href={PAGE_FILES.rulings}>
          <span className="count__n">{pending.length}</span>
          <span className="count__label">待裁决</span>
          <span className="count__sub">{high > 0 ? `高优先 ${high}` : " "}</span>
        </a>
        <a className="count" href={PAGE_FILES.audits}>
          <span className="count__n">{audits}</span>
          <span className="count__label">待抽查</span>
          <span className="count__sub">{" "}</span>
        </a>
        <a className="count" href={PAGE_FILES.dise}>
          <span className="count__n">{entries.length}</span>
          <span className="count__label">条底色</span>
          <span className="count__sub">
            已确认 {confirmed}
            {drafts > 0 ? ` · 草稿 ${drafts}` : ""}
          </span>
        </a>
      </div>
      <ul className="links">
        <li>
          <a href={PAGE_FILES.dise}>
            整张底色 <span>{entries.length} 条</span>
          </a>
        </li>
        <li>
          <a href={PAGE_FILES.rulings}>
            待裁决 <span>{pending.length} 项</span>
          </a>
        </li>
        <li>
          <a href={PAGE_FILES.audits}>
            待抽查 <span>{audits} 条</span>
          </a>
        </li>
        <li>
          <a href={PAGE_FILES.precedents}>
            判例 <span>{model.precedents.length} 个</span>
          </a>
        </li>
        <li>
          <a href={PAGE_FILES.report}>
            最近入库 <span>{latest ? formatIsoDate(latest.date, { withYear: false }) || latest.name : "还没有"}</span>
          </a>
        </li>
      </ul>
      {latest?.resultLine ? (
        <p className="lede">
          最近一次入库：<Inline resolve={resolve}>{latest.resultLine}</Inline>
        </p>
      ) : null}
    </>,
  );
}

/** One line of Markdown without a paragraph around it. */
function Inline({ children, resolve }: { children: string; resolve: Resolve }) {
  const plugins = [remarkGfm, [remarkWikilinks, { resolve }]] as NonNullable<
    ComponentProps<typeof ReactMarkdown>["remarkPlugins"]
  >;
  return (
    <ReactMarkdown remarkPlugins={plugins} components={{ ...markdownComponents, p: ({ children: inner }) => <>{inner}</> }}>
      {children}
    </ReactMarkdown>
  );
}

/** The whole 底色, section by section. */
export function renderDise(model: KbModel): string {
  const resolve = exportLinkResolver(model);
  const dise = model.dise;
  if (!dise) {
    return render("dise", "底色", <p className="empty">知识库里还没有底色</p>);
  }
  const version = fmString(dise.frontmatter, "版本");
  const updated = fmString(dise.frontmatter, "更新");
  const status = fmString(dise.frontmatter, "状态");
  const confirmed = dise.entries.filter((e) => e.confirmed).length;
  const drafts = dise.entries.filter((e) => e.draft).length;
  return render(
    "dise",
    "底色",
    <>
      <Head
        eyebrow="底色"
        title={dise.title ?? "我的决策底色"}
        meta={[
          version,
          updated ? `更新 ${updated}` : null,
          status,
          `${dise.entries.length} 条 · 已确认 ${confirmed}${drafts > 0 ? ` · 草稿 ${drafts}` : ""}`,
        ]
          .filter(Boolean)
          .join(" · ")}
      >
        {dise.introMarkdown ? (
          <div className="lede">
            <Md resolve={resolve}>{dise.introMarkdown}</Md>
          </div>
        ) : null}
      </Head>
      {dise.sections.map((section, index) => (
        <section key={`${section.heading}-${index}`} className="section">
          <div className="section__head">
            <span className="section__ordinal">{section.ordinal ?? chineseNumber(index + 1)}</span>
            <h2 className="section__title">{section.title || "其他"}</h2>
            <span className="section__count">{section.entries.length} 条</span>
          </div>
          {section.introMarkdown ? (
            <div className="small" style={{ marginBottom: 12 }}>
              <Md resolve={resolve}>{section.introMarkdown}</Md>
            </div>
          ) : null}
          {section.entries.map((entry) => (
            <EntryCard key={entry.id} entry={entry} resolve={resolve} />
          ))}
        </section>
      ))}
    </>,
  );
}

function PrecedentCard({ precedent, resolve }: { precedent: Precedent; resolve: Resolve }) {
  return (
    <article className="card" id={anchorId(precedent.id)}>
      <div className="card__head">
        <span className="card__id num">{precedent.id}</span>
        <h3 className="card__title">{precedent.title}</h3>
      </div>
      <div className="chips">
        {precedent.status ? <span className="chip">{precedent.status}</span> : null}
        {precedent.rulingDate ? <span className="chip">裁决 {precedent.rulingDate}</span> : null}
        {precedent.related.map((id) => (
          <a key={id} className="chip" href={`${PAGE_FILES.dise}#${anchorId(id)}`}>
            {id}
          </a>
        ))}
      </div>
      <div style={{ marginTop: 10 }}>
        <Md resolve={resolve}>{sectionsMarkdown(precedent.sections)}</Md>
      </div>
    </article>
  );
}

/** Every precedent. */
export function renderPrecedents(model: KbModel): string {
  const resolve = exportLinkResolver(model);
  return render(
    "precedents",
    "判例",
    <>
      <Head eyebrow="判例" title="判例" meta={`${model.precedents.length} 个 · 你裁决过的冲突`} />
      {model.precedents.length > 0 ? (
        <nav className="toc">
          {model.precedents.map((p) => (
            <a key={p.path} href={`#${anchorId(p.id)}`}>
              {p.id}
            </a>
          ))}
        </nav>
      ) : (
        <p className="empty">还没有判例</p>
      )}
      {model.precedents.map((precedent) => (
        <PrecedentCard key={precedent.path} precedent={precedent} resolve={resolve} />
      ))}
    </>,
  );
}

/** How to answer a pending ruling on the phone. */
function AnswerHelp() {
  return (
    <aside className="box">
      <h2>在 iPhone 上回答</h2>
      <ol>
        <li>运行快捷指令「答裁决」，选一个问题，读完后打字或按麦克风说出你的回答。</li>
        <li>没装快捷指令时，回到 Mac 上的底色里回答也一样。</li>
      </ol>
      <p className="small" style={{ marginTop: 6 }}>
        Mac 上的底色会把回答原样收进裁决收件箱，等下次落裁决。
      </p>
    </aside>
  );
}

function RulingCard({ ruling, resolve }: { ruling: Ruling; resolve: Resolve }) {
  return (
    <article className="card" id={anchorId(ruling.id)}>
      <div className="card__head">
        <span className="card__id num">{ruling.id}</span>
        <h3 className="card__title">{ruling.title}</h3>
      </div>
      <div className="chips">
        {rulingChips(ruling).map((chip) => (
          <span key={chip.text} className={chip.high ? "chip is-high" : "chip"}>
            {chip.text}
          </span>
        ))}
      </div>
      <div style={{ marginTop: 10 }}>
        <Md resolve={resolve}>{sectionsMarkdown(ruling.sections)}</Md>
      </div>
      <p className="small" style={{ marginTop: 10 }}>
        回答：快捷指令「答裁决」里选 <span className="num">{ruling.id}</span>
      </p>
    </article>
  );
}

/** The open rulings with their materials and questions. */
export function renderRulings(model: KbModel): string {
  const resolve = exportLinkResolver(model);
  const pending = model.pendingRulings;
  return render(
    "rulings",
    "待裁决",
    <>
      <Head eyebrow="待裁决" title="要你定的事" meta={`${pending.length} 项，按优先级排`} />
      <AnswerHelp />
      {pending.length > 0 ? (
        <nav className="toc">
          {pending.map((r) => (
            <a key={r.path} href={`#${anchorId(r.id)}`}>
              {r.id}
            </a>
          ))}
        </nav>
      ) : (
        <p className="empty">现在没有待裁决的问题</p>
      )}
      {pending.map((ruling) => (
        <RulingCard key={ruling.path} ruling={ruling} resolve={resolve} />
      ))}
    </>,
  );
}

/** The unchecked 待你抽查 items of every report. */
export function renderAudits(model: KbModel): string {
  const resolve = exportLinkResolver(model);
  const groups = model.reports
    .map((report) => ({ report, items: report.auditItems.filter((item) => !item.checked) }))
    .filter((group) => group.items.length > 0);
  const total = groups.reduce((sum, group) => sum + group.items.length, 0);
  return render(
    "audits",
    "待抽查",
    <>
      <Head eyebrow="待抽查" title="AI 按判例做的裁决" meta={`${total} 条等你看一眼`}>
        <p className="lede">认可或不认可要在 Mac 上的底色里做；这里只能看。</p>
      </Head>
      {groups.length === 0 ? <p className="empty">没有要抽查的</p> : null}
      {groups.map(({ report, items }) => (
        <section key={report.path} className="section">
          <div className="section__head">
            <h2 className="section__title">{reportLabel(report)}</h2>
            <span className="section__count">{items.length} 条</span>
          </div>
          {items.map((item, index) => {
            const precedent = item.precedent ? resolve(item.precedent) : null;
            return (
              <article key={`${item.itemId ?? index}-${item.line}`} className="card" id={anchorId(`${report.date ?? report.name}-${item.itemId ?? index}`)}>
                <div className="card__head">
                  {item.itemId ? <span className="card__id num">{item.itemId}</span> : null}
                  <h3 className="card__title">
                    <Inline resolve={resolve}>{item.summary || item.text}</Inline>
                  </h3>
                </div>
                {item.tension || item.ruling || item.precedent ? (
                  <dl className="audit__parts">
                    {item.tension ? (
                      <>
                        <dt>张力</dt>
                        <dd>
                          <Inline resolve={resolve}>{item.tension}</Inline>
                        </dd>
                      </>
                    ) : null}
                    {item.ruling ? (
                      <>
                        <dt>裁决</dt>
                        <dd>
                          <Inline resolve={resolve}>{item.ruling}</Inline>
                        </dd>
                      </>
                    ) : null}
                    {item.precedent ? (
                      <>
                        <dt>依据</dt>
                        <dd>{precedent ? <a href={precedent.href}>{item.precedent}</a> : item.precedent}</dd>
                      </>
                    ) : null}
                  </dl>
                ) : null}
              </article>
            );
          })}
        </section>
      ))}
    </>,
  );
}

function reportLabel(report: Report): string {
  if (!report.date) return report.name;
  const nth = report.sequence > 1 ? `第 ${report.sequence} 次` : "";
  return `${formatIsoDate(report.date)}${nth}的入库`;
}

/**
 * The report sections that go to the phone, by their exact `##` heading.
 * Anything not listed here stays on the Mac, whatever words its heading has.
 */
const PHONE_SECTIONS: readonly ((heading: string) => boolean)[] = [
  (heading) => heading.startsWith("待你抽查"),
  (heading) => heading === "交给你裁决",
  (heading) => heading === "给已有待裁决补的材料",
];

/** True for a report section that goes to the phone. */
export function isPhoneSection(heading: string): boolean {
  const trimmed = heading.trim();
  return PHONE_SECTIONS.some((allowed) => allowed(trimmed));
}

/** The sections of a report that go to the phone, and whether others stay on the Mac. */
export function reportSectionsForPhone(report: Report): { shown: { heading: string; markdown: string }[]; hidden: boolean } {
  const { sections } = splitSections(report.body, 2);
  const shown = sections.filter((s) => isPhoneSection(s.heading));
  return { shown, hidden: shown.length < sections.length };
}

/** The latest ingest report, as far as it is about the 底色 layer. */
export function renderReport(model: KbModel): string {
  const resolve = exportLinkResolver(model);
  const report = model.reports[0];
  if (!report) {
    return render("report", "最近入库", <p className="empty">还没有入库报告</p>);
  }
  const { shown, hidden } = reportSectionsForPhone(report);
  return render(
    "report",
    "最近入库",
    <>
      <Head eyebrow="最近入库" title={report.title ?? reportLabel(report)} meta={report.date ? formatIsoDate(report.date) : undefined} />
      {report.resultParts.length > 0 ? (
        <div className="chips" style={{ marginBottom: 18 }}>
          {report.resultParts.map((part) => (
            <span key={part} className="chip">
              <Inline resolve={resolve}>{part}</Inline>
            </span>
          ))}
        </div>
      ) : null}
      {shown.length > 0 ? (
        <article className="card">
          <Md resolve={resolve}>{sectionsMarkdown(shown)}</Md>
        </article>
      ) : (
        <p className="empty">这次入库没有要你看的裁决</p>
      )}
      {hidden ? <p className="small">报告的其余部分只在 Mac 上看。</p> : null}
    </>,
  );
}

/** A guide page in the same look (the bridge folder's 快捷指令/使用说明.html). */
export function renderGuide(markdown: string): string {
  const none: Resolve = () => null;
  const body = (
    <>
      <Head eyebrow="iPhone" title="在 iPhone 上用底色" meta="只用「快捷指令」和「文件」两个自带的 App" />
      <article className="card">
        <Md resolve={none}>{markdown}</Md>
      </article>
    </>
  );
  // The guide lives in 快捷指令/, away from the exported pages: no navigation.
  return `<!doctype html>\n${renderToStaticMarkup(
    <html lang="zh-CN">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover" />
        <meta name="color-scheme" content="light dark" />
        <meta httpEquiv="Content-Security-Policy" content={CSP} />
        <title>底色 · 在 iPhone 上用底色</title>
        <style dangerouslySetInnerHTML={{ __html: PHONE_CSS }} />
      </head>
      <body>
        <div className="wrap">
          <header className="mast">
            <div className="mast__row">
              <span className="seal">底色</span>
              <span className="mast__title">使用说明</span>
            </div>
          </header>
          <main>{body}</main>
          <footer className="foot">这份说明由 Mac 上的「底色」放在这里</footer>
        </div>
      </body>
    </html>,
  )}\n`;
}
