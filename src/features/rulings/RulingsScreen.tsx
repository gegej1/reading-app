import { Link, useSearchParams } from "react-router";
import { stripMarkdown, type Report, type Ruling } from "../../domain";
import { useKb } from "../../app/KbContext";
import { pageRoute } from "../../app/links";
import { RunStartButton } from "../../app/RunStartButton";
import { Chip, cx, EmptyState, Numbered, PageHeader } from "../../ui/components";
import { formatIsoDate } from "../../ui/format";
import { AuditChecklist } from "./AuditChecklist";

const PRIORITY_GROUPS: { key: string; label: string }[] = [
  { key: "高", label: "高优先级" },
  { key: "中", label: "中优先级" },
  { key: "低", label: "低优先级" },
];

function groupByPriority(rulings: readonly Ruling[]) {
  const known = PRIORITY_GROUPS.map((group) => ({
    ...group,
    rulings: rulings.filter((r) => r.priority === group.key),
  }));
  const other = rulings.filter((r) => !PRIORITY_GROUPS.some((g) => g.key === r.priority));
  if (other.length > 0) known.push({ key: "other", label: "未标优先级", rulings: other });
  return known.filter((group) => group.rulings.length > 0);
}

function RulingRow({ ruling, index }: { ruling: Ruling; index: number }) {
  const { model } = useKb();
  const answers = model.answerIndex.rulings.get(ruling.id)?.length ?? 0;
  return (
    <li className="rise" style={{ animationDelay: `${Math.min(index, 10) * 35}ms` }}>
      <Link to={pageRoute(ruling.path)} className="ruling-row">
        <span className={cx("ruling-row__priority", ruling.priority === "高" && "is-high")} aria-hidden="true">
          {ruling.priority ?? "·"}
        </span>
        <span className="ruling-row__id">{ruling.id}</span>
        <span className="ruling-row__main">
          <span className="ruling-row__title">{ruling.title}</span>
          {ruling.question ? (
            <span className="ruling-row__question">{stripMarkdown(ruling.question)}</span>
          ) : null}
        </span>
        <span className="ruling-row__meta">
          {answers > 0 ? (
            <Chip tone="jade" title={answers > 1 ? `已经交上 ${answers} 份回答` : undefined}>
              已回答，待落裁决
            </Chip>
          ) : null}
          {ruling.category ? <Chip tone={ruling.category === "新冲突" ? "seal" : "plain"}>{ruling.category}</Chip> : null}
          {ruling.updated ? <span className="ruling-row__date">{formatIsoDate(ruling.updated, { withYear: false })}</span> : null}
        </span>
      </Link>
    </li>
  );
}

function PendingList({ rulings }: { rulings: readonly Ruling[] }) {
  if (rulings.length === 0) {
    return <EmptyState title="没有待裁决的问题">新的冲突会在入库时出现在这里。</EmptyState>;
  }
  let index = 0;
  return (
    <div className="ruling-groups">
      {groupByPriority(rulings).map((group) => (
        <section key={group.key} className="ruling-group" aria-label={group.label}>
          <h2 className={cx("ruling-group__title", group.key === "高" && "is-high")}>
            {group.label}
            <span className="ruling-group__count num">{group.rulings.length}</span>
          </h2>
          <ul className="ruling-list">
            {group.rulings.map((ruling) => (
              <RulingRow key={ruling.path} ruling={ruling} index={index++} />
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

function AuditList({ reports }: { reports: readonly Report[] }) {
  const withItems = reports.filter((r) => r.auditItems.length > 0);
  if (withItems.length === 0) {
    return <EmptyState title="没有待抽查的条目">AI 按判例做的裁决会列在入库报告里。</EmptyState>;
  }
  return (
    <div className="audit-groups">
      {withItems.map((report) => {
        const open = report.auditItems.filter((i) => !i.checked).length;
        return (
          <section key={report.path} className="audit-group rise" aria-label={report.title ?? report.name}>
            <header className="audit-group__head">
              <h2 className="audit-group__title">
                <Link to={pageRoute(report.path)}>{report.title ?? report.name}</Link>
              </h2>
              <span className="audit-group__meta">
                {report.date ? formatIsoDate(report.date) : null}
                <span>
                  待抽查 <span className="num">{open}</span> / <span className="num">{report.auditItems.length}</span>
                </span>
              </span>
            </header>
            {report.resultParts.length > 0 ? (
              <ul className="audit-group__result" aria-label="入库结果">
                {report.resultParts.map((part) => (
                  <li key={part}>
                    <Numbered text={part} />
                  </li>
                ))}
              </ul>
            ) : null}
            <AuditChecklist report={report} />
          </section>
        );
      })}
    </div>
  );
}

export function RulingsScreen() {
  const { model } = useKb();
  const [params, setParams] = useSearchParams();
  const tab = params.get("tab") === "audit" ? "audit" : "pending";
  const unchecked = model.dashboard.uncheckedAudit;
  const pending = model.pendingRulings;

  return (
    <div className="screen rulings">
      <PageHeader
        eyebrow="裁决"
        title="裁决台"
        meta={
          <>
            <span>
              待裁决 <span className="num">{pending.length}</span> 项（高优先级{" "}
              <span className="num">{model.dashboard.pendingRulings.high}</span>）
            </span>
            <span>
              待抽查 <span className="num">{unchecked}</span> 条
            </span>
            {model.dashboard.answeredRulings > 0 ? (
              <span>
                已回答 <span className="num">{model.dashboard.answeredRulings}</span> 份，待落裁决
              </span>
            ) : null}
          </>
        }
        actions={<RunStartButton task="landRulings" count={model.dashboard.answeredRulings} />}
      />

      <div className="tabs" role="tablist" aria-label="裁决分类">
        <button
          type="button"
          role="tab"
          aria-selected={tab === "pending"}
          className={cx("tab", tab === "pending" && "is-active")}
          onClick={() => setParams({}, { replace: true })}
        >
          待裁决 <span className="tab__count num">{pending.length}</span>
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={tab === "audit"}
          className={cx("tab", tab === "audit" && "is-active")}
          onClick={() => setParams({ tab: "audit" }, { replace: true })}
        >
          待抽查 <span className="tab__count num">{unchecked}</span>
        </button>
      </div>

      <div role="tabpanel" className="rulings__panel">
        {tab === "pending" ? <PendingList rulings={pending} /> : <AuditList reports={model.reports} />}
      </div>
    </div>
  );
}
