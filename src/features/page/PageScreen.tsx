import { Link, useParams } from "react-router";
import {
  reportSegments,
  stripLeadingH1,
  stripMarkdown,
  type KbDoc,
  type KbModel,
  type LinkRef,
  type Report,
} from "../../domain";
import { useKb } from "../../app/KbContext";
import {
  docRoute,
  findPage,
  groupLabel,
  NAV_ITEMS,
  navSectionForGroup,
  sameNamePaths,
} from "../../app/links";
import { Chip, EmptyState } from "../../ui/components";
import { ExternalLink, isExternalUrl } from "../../ui/ExternalLink";
import { Markdown } from "../../ui/Markdown";
import { AnswerBox } from "../rulings/AnswerBox";
import { AuditChecklist } from "../rulings/AuditChecklist";

/** Frontmatter values as short display strings. */
function frontmatterEntries(data: Record<string, unknown>): [string, string][] {
  return Object.entries(data)
    .map(([key, value]): [string, string] => {
      if (Array.isArray(value)) return [key, value.map(String).join("、")];
      if (value === null || value === undefined) return [key, ""];
      if (typeof value === "object") return [key, JSON.stringify(value)];
      return [key, String(value)];
    })
    .filter(([, value]) => value.trim() !== "");
}

/** The source line of a backlink, as plain text. */
function excerpt(model: KbModel, ref: LinkRef): string {
  const source = model.byPath.get(ref.from);
  const line = source?.content.split("\n")[ref.link.line] ?? "";
  const text = stripMarkdown(line);
  return text.length > 90 ? `${text.slice(0, 90)}…` : text;
}

function Backlinks({ doc }: { doc: KbDoc }) {
  const { model } = useKb();
  const refs = model.links.backlinks.get(doc.path) ?? [];
  return (
    <section className="backlinks" aria-label="反向链接">
      <h2 className="backlinks__title">
        引用了这一页的地方<span className="backlinks__count num">{refs.length}</span>
      </h2>
      {refs.length === 0 ? (
        <p className="muted">还没有别的页面链接到这里。</p>
      ) : (
        <ul className="backlinks__list">
          {refs.map((ref) => {
            const from = model.byPath.get(ref.from);
            if (!from) return null;
            return (
              <li key={ref.from}>
                <Link to={docRoute(from)} className="backlink">
                  <span className="backlink__head">
                    <span className="backlink__title">{from.title}</span>
                    <Chip>{groupLabel(from)}</Chip>
                  </span>
                  <span className="backlink__excerpt">{excerpt(model, ref)}</span>
                </Link>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

/**
 * Shown when other files share this page's name. Bare wikilinks to that name
 * open the first path in code-point order; this lists the others.
 */
function SameNameNote({ doc, paths }: { doc: KbDoc; paths: string[] }) {
  const { model } = useKb();
  const opened = model.links.index.resolve(doc.name);
  return (
    <aside className="namesakes" aria-label="同名文件">
      <p className="namesakes__title">同名文件</p>
      <p className="namesakes__text">
        知识库里还有 <span className="num">{paths.length}</span> 个文件也叫「{doc.name}」。只写名字的双链
        <code>[[{doc.name}]]</code>
        {opened === doc.path ? "打开的是这一页" : <>打开的是 <code>{opened}</code></>}
        ；要指向别的文件，请在双链里写上路径。
      </p>
      <ul className="namesakes__list">
        {paths.map((path) => {
          const other = model.byPath.get(path);
          return other ? (
            <li key={path}>
              <Link to={docRoute(other)} className="text-link">
                {path}
              </Link>
            </li>
          ) : null;
        })}
      </ul>
    </aside>
  );
}

/** A report: its Markdown, with the 待你抽查 checklist as controls. */
function ReportBody({ report }: { report: Report }) {
  const { resolveLink } = useKb();
  return (
    <div className="page__body report-body">
      {reportSegments(stripLeadingH1(report.body)).map((segment, index) =>
        segment.kind === "markdown" ? (
          <Markdown key={index} resolve={resolveLink}>
            {segment.markdown}
          </Markdown>
        ) : (
          <section key={index} className="report-audit" aria-label={segment.heading}>
            <h2 className="report-audit__title">{segment.heading}</h2>
            <AuditChecklist report={report} />
            {segment.rest ? <Markdown resolve={resolveLink}>{segment.rest}</Markdown> : null}
          </section>
        ),
      )}
    </div>
  );
}

export function PageScreen() {
  const { model, resolveLink } = useKb();
  const params = useParams();
  // The splat holds the repo path, already decoded segment by segment.
  const requested = params["*"] ?? "";
  const doc = findPage(model, requested);

  if (!doc) {
    return (
      <div className="screen page">
        <EmptyState title={`找不到「${requested}」`}>
          这个页面可能改名或删除了。<Link to="/knowledge">回到书架</Link>
        </EmptyState>
      </div>
    );
  }

  const section = navSectionForGroup(doc.group);
  const sectionItem = NAV_ITEMS.find((item) => item.section === section);
  const chips = frontmatterEntries(doc.frontmatter);
  const namesakes = sameNamePaths(model, doc);
  const ruling = doc.group === "ruling" ? model.rulings.find((candidate) => candidate.path === doc.path) : undefined;
  const report = doc.group === "report" ? model.reports.find((candidate) => candidate.path === doc.path) : undefined;
  const answered = ruling ? (model.answerIndex.rulings.get(ruling.id)?.length ?? 0) : 0;

  return (
    <article className="screen page">
      <nav className="crumbs" aria-label="位置">
        {sectionItem ? <Link to={sectionItem.to}>{sectionItem.label}</Link> : null}
        <span aria-hidden="true">／</span>
        <span>{groupLabel(doc)}</span>
      </nav>
      <h1 className="page__title">
        {doc.title}
        {ruling && ruling.pending && answered > 0 ? (
          <span className="page__status">
            <Chip tone="jade">已回答，待落裁决</Chip>
          </span>
        ) : null}
      </h1>
      {chips.length > 0 ? (
        <dl className="props" aria-label="页面属性">
          {chips.map(([key, value]) => (
            <div className="prop" key={key}>
              <dt>{key}</dt>
              <dd>
                {isExternalUrl(value) && !/\s/.test(value) ? (
                  <ExternalLink href={value}>{value}</ExternalLink>
                ) : (
                  value
                )}
              </dd>
            </div>
          ))}
        </dl>
      ) : null}
      {namesakes.length > 0 ? <SameNameNote doc={doc} paths={namesakes} /> : null}
      {report && report.auditItems.length > 0 ? (
        <ReportBody report={report} />
      ) : (
        <Markdown resolve={resolveLink} className="page__body">
          {stripLeadingH1(doc.body)}
        </Markdown>
      )}
      {ruling?.pending ? <AnswerBox ruling={ruling} /> : null}
      <Backlinks doc={doc} />
      <p className="page__path">{doc.path}</p>
    </article>
  );
}
