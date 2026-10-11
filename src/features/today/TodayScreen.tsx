import { Link } from "react-router";
import { pickDailyEntry, stripMarkdown, type DiseEntry, type DiseSection } from "../../domain";
import { useKb } from "../../app/KbContext";
import { diseEntryRoute, pageRoute } from "../../app/links";
import { cx, EmptyState, Hint, Numbered, Seal } from "../../ui/components";
import { formatIsoDate, todayLabel } from "../../ui/format";
import { useVerticalLetter } from "./useVerticalLetter";

function LedgerCell(props: {
  label: string;
  value: number;
  unit: string;
  note: string;
  to?: string;
  accent?: boolean;
  delay: number;
}) {
  const body = (
    <>
      <span className="ledger__label">{props.label}</span>
      <span className="ledger__value">
        <span className="ledger__number">{props.value}</span>
        <span className="ledger__unit">{props.unit}</span>
      </span>
      <span className={cx("ledger__note", props.accent && "is-accent")}>{props.note}</span>
    </>
  );
  const style = { animationDelay: `${props.delay}ms` };
  return props.to ? (
    <Link to={props.to} className="ledger__cell is-link rise" style={style}>
      {body}
    </Link>
  ) : (
    <div className="ledger__cell rise" style={style}>
      {body}
    </div>
  );
}

/** The letter paper: vertical when it fits (see useVerticalLetter), else horizontal. */
function LetterPaper({ entry }: { entry: DiseEntry }) {
  const quote = stripMarkdown(entry.quote) || entry.title;
  const { vertical, paperRef } = useVerticalLetter(quote);
  return (
    <div ref={paperRef} className={cx("letter__paper", vertical ? "is-vertical" : "is-horizontal")}>
      <blockquote className="letter__quote">{quote}</blockquote>
      <footer className="letter__sign">
        <span className="letter__dash" aria-hidden="true">
          ——
        </span>
        <span className="letter__id">{entry.id}</span>
        <span className="letter__title">{entry.title}</span>
        {entry.confirmed ? (
          <span className="letter__seal">
            <Seal size="sm" />
          </span>
        ) : null}
      </footer>
    </div>
  );
}

function DailyLetter({ entry, section }: { entry: DiseEntry | null; section: DiseSection | null }) {
  if (!entry) {
    return (
      <article className="letter rise" style={{ animationDelay: "180ms" }}>
        <p className="card-label">今日一条底色</p>
        <EmptyState title="还没有底色条目">
          在知识库的 <code>self/底色.md</code> 里写下第一条 <code>### D01 标题</code>。
        </EmptyState>
      </article>
    );
  }
  return (
    <article className="letter rise" style={{ animationDelay: "180ms" }} aria-label="今日一条底色">
      <p className="card-label">
        今日一条底色
        {section?.heading ? <span className="card-label__aside">{section.heading}</span> : null}
      </p>
      <LetterPaper key={entry.id} entry={entry} />
      <Link to={diseEntryRoute(entry.id)} className="text-link letter__more">
        在底色里看这一条 →
      </Link>
    </article>
  );
}

export function TodayScreen({ now = new Date() }: { now?: Date }) {
  const { model } = useKb();
  const { dashboard } = model;
  const label = todayLabel(now);
  const entry = model.dise ? pickDailyEntry(model.dise.entries, now) : null;
  const section = entry && model.dise ? (model.dise.sections[entry.sectionIndex] ?? null) : null;
  const report = dashboard.latestReport;
  const pending = dashboard.pendingRulings;

  return (
    <div className="screen today">
      <header className="today__head">
        <div className="today__date">
          <h1 className="today__md">{label.monthDay}</h1>
          <p className="today__meta">
            <span className="num">{label.year}</span> · {label.weekday}
          </p>
        </div>
        <Hint text="还没开放：随手记一笔，直接进收件箱" align="end">
          <button type="button" className="btn btn--quiet" disabled>
            ✎ 记一笔
          </button>
        </Hint>
      </header>

      <section className="ledger" aria-label="待办">
        <LedgerCell
          label="收件箱"
          value={dashboard.inboxCaptures}
          unit="条随手记"
          note={
            dashboard.answeredRulings > 0
              ? `另有 ${dashboard.answeredRulings} 份回答待落裁决`
              : "等下一次入库处理"
          }
          to="/raw"
          delay={0}
        />
        <LedgerCell
          label="待裁决"
          value={pending.total}
          unit="项"
          note={pending.high > 0 ? `其中高优先级 ${pending.high} 项` : "没有高优先级"}
          accent={pending.high > 0}
          to="/rulings"
          delay={60}
        />
        <LedgerCell
          label="待抽查"
          value={dashboard.uncheckedAudit}
          unit="条"
          note="AI 按判例做的裁决"
          accent={dashboard.uncheckedAudit > 0}
          to="/rulings?tab=audit"
          delay={120}
        />
      </section>

      <div className="today__grid">
        <DailyLetter entry={entry} section={section} />

        <aside className="today__side">
          <section className="report-card rise" style={{ animationDelay: "240ms" }} aria-label="最近一次入库">
            <p className="card-label">最近一次入库</p>
            {report ? (
              <>
                <h2 className="report-card__title">
                  <Link to={pageRoute(report.path)}>{report.title ?? report.name}</Link>
                </h2>
                {report.date ? <p className="report-card__date">{formatIsoDate(report.date)}</p> : null}
                {report.resultParts.length > 0 ? (
                  <ul className="report-card__parts">
                    {report.resultParts.map((part) => (
                      <li key={part}>
                        <Numbered text={part} />
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="muted">报告里没有"结果"一行。</p>
                )}
              </>
            ) : (
              <EmptyState title="还没有入库报告" />
            )}
          </section>

          <div className="today__actions rise" style={{ animationDelay: "300ms" }}>
            <Hint text="还没开放：在 App 里发起入库">
              <button type="button" className="btn btn--seal" disabled>
                入库 ▶
              </button>
            </Hint>
            <Link to="/rulings" className="btn">
              去裁决
            </Link>
            <Link to="/rulings?tab=audit" className="btn">
              去抽查
            </Link>
          </div>
        </aside>
      </div>
    </div>
  );
}
