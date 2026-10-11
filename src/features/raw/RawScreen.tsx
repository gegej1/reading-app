import { useMemo } from "react";
import { Link } from "react-router";
import { countRawItems, fmString } from "../../domain";
import { useKb } from "../../app/KbContext";
import { docRoute } from "../../app/links";
import { EmptyState, PageHeader } from "../../ui/components";
import { formatIsoDate } from "../../ui/format";
import { captureSummary, docDate, groupRaw } from "./rawGroups";

export function RawScreen() {
  const { model } = useKb();
  const groups = useMemo(() => groupRaw(model.raw), [model]);
  const captures = model.captures;

  return (
    <div className="screen raw">
      <PageHeader
        eyebrow="原料"
        title="原料箱"
        meta={
          <span>
            逐字保存的原文，只增不改 · 共 <span className="num">{model.raw.length}</span> 份
          </span>
        }
      />

      {captures.length > 0 ? (
        <section className="raw-group raw-group--queue rise" aria-label="收件箱队列">
          <h2 className="raw-group__title">
            待入库的随手记
            <span className="raw-group__count num">{captures.length}</span>
          </h2>
          <p className="raw-group__note">还在 inbox/ 里，下一次入库时逐字存进原料。</p>
          <ul className="raw-list">
            {captures.map((doc) => {
              const time = fmString(doc.frontmatter, "时间");
              const source = fmString(doc.frontmatter, "来源");
              return (
                <li key={doc.path}>
                  <Link to={docRoute(doc)} className="raw-item">
                    <span className="raw-item__date">{time ?? "—"}</span>
                    <span className="raw-item__main">
                      <span className="raw-item__capture">{captureSummary(doc) || doc.name}</span>
                      {source ? <span className="raw-item__source">来自 {source}</span> : null}
                    </span>
                    <span className="raw-item__count" />
                  </Link>
                </li>
              );
            })}
          </ul>
        </section>
      ) : null}

      {groups.length === 0 ? (
        <EmptyState title="还没有原料">入库时，原文会逐字存进 raw/。</EmptyState>
      ) : (
        groups.map((group, groupIndex) => (
          <section
            key={group.key || "root"}
            className="raw-group rise"
            style={{ animationDelay: `${(groupIndex + 1) * 60}ms` }}
            aria-label={group.label}
          >
            <h2 className="raw-group__title">
              {group.label}
              <span className="raw-group__count num">{group.docs.length}</span>
            </h2>
            <ul className="raw-list">
              {group.docs.map((doc) => {
                const items = countRawItems(doc.body);
                const source = fmString(doc.frontmatter, "来源");
                const date = docDate(doc);
                return (
                  <li key={doc.path}>
                    <Link to={docRoute(doc)} className="raw-item">
                      <span className="raw-item__date">{date ? formatIsoDate(date, { withYear: false }) : "—"}</span>
                      <span className="raw-item__main">
                        <span className="raw-item__title">{doc.title}</span>
                        {source ? <span className="raw-item__source">{source}</span> : null}
                      </span>
                      <span className="raw-item__count">
                        {items > 0 ? (
                          <>
                            <span className="num">{items}</span> 条
                          </>
                        ) : null}
                      </span>
                    </Link>
                  </li>
                );
              })}
            </ul>
          </section>
        ))
      )}
    </div>
  );
}
