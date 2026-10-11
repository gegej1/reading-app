import { Link, useSearchParams } from "react-router";
import { stripMarkdown, type AuditItem, type Report, type Ruling } from "../../domain";
import { useKb } from "../../app/KbContext";
import { docRoute, pageRoute } from "../../app/links";
import { Chip, cx, EmptyState, Numbered, PageHeader } from "../../ui/components";
import { formatIsoDate } from "../../ui/format";

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

function AuditRow({ item }: { item: AuditItem }) {
  const { model } = useKb();
  const precedentPath = item.precedent ? model.links.index.resolve(item.precedent) : null;
  const precedent = precedentPath ? model.byPath.get(precedentPath) : undefined;
  return (
    <li className={cx("audit-row", item.checked && "is-checked")}>
      <span className={cx("audit-row__box", item.checked && "is-checked")} role="img" aria-label={item.checked ? "已认可" : "未抽查"}>
        {item.checked ? "✓" : ""}
      </span>
      <div className="audit-row__main">
        <p className="audit-row__summary">
          {item.itemId ? <span className="audit-row__id">{item.itemId}</span> : null}
          {item.summary}
        </p>
        {item.tension ? (
          <p className="audit-row__tension">
            <span className="audit-row__arrow" aria-label="对照">↔</span> {item.tension}
          </p>
        ) : null}
        {item.ruling ? (
          <p className="audit-row__ruling">
            <span className="audit-row__label">AI 裁决</span>
            {item.ruling}
          </p>
        ) : null}
        {item.precedent ? (
          <p className="audit-row__basis">
            依据{" "}
            {precedent ? (
              <Link to={docRoute(precedent)} className="text-link">
                {precedent.name}
              </Link>
            ) : (
              <span className="wikilink--broken">{item.precedent}</span>
            )}
          </p>
        ) : null}
      </div>
      {item.checked ? <span className="audit-row__state">已认可</span> : null}
    </li>
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
            <ul className="audit-list">
              {report.auditItems.map((item) => (
                <AuditRow key={`${item.line}-${item.text}`} item={item} />
              ))}
            </ul>
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
            <Chip tone="ink" title="回答裁决和勾选抽查会在后续版本开放">
              M1 只读
            </Chip>
          </>
        }
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
