import { Link, useParams } from "react-router";
import { stripLeadingH1, stripMarkdown, type KbDoc, type KbModel, type LinkRef } from "../../domain";
import { useKb } from "../../app/KbContext";
import { docRoute, groupLabel, NAV_ITEMS, navSectionForGroup, safeDecode } from "../../app/links";
import { Chip, EmptyState } from "../../ui/components";
import { Markdown } from "../../ui/Markdown";

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

export function PageScreen() {
  const { model, resolveLink } = useKb();
  const params = useParams();
  const name = safeDecode(params.name);
  const doc = model.byName.get(name);

  if (!doc) {
    return (
      <div className="screen page">
        <EmptyState title={`找不到「${name}」`}>
          这个页面可能改名或删除了。<Link to="/knowledge">回到书架</Link>
        </EmptyState>
      </div>
    );
  }

  const section = navSectionForGroup(doc.group);
  const sectionItem = NAV_ITEMS.find((item) => item.section === section);
  const chips = frontmatterEntries(doc.frontmatter);

  return (
    <article className="screen page">
      <nav className="crumbs" aria-label="位置">
        {sectionItem ? <Link to={sectionItem.to}>{sectionItem.label}</Link> : null}
        <span aria-hidden="true">／</span>
        <span>{groupLabel(doc)}</span>
      </nav>
      <h1 className="page__title">{doc.title}</h1>
      {chips.length > 0 ? (
        <dl className="props" aria-label="页面属性">
          {chips.map(([key, value]) => (
            <div className="prop" key={key}>
              <dt>{key}</dt>
              <dd>
                {/^https?:\/\/\S+$/.test(value) ? (
                  <a href={value} target="_blank" rel="noreferrer noopener">
                    {value}
                  </a>
                ) : (
                  value
                )}
              </dd>
            </div>
          ))}
        </dl>
      ) : null}
      <Markdown resolve={resolveLink} className="page__body">
        {stripLeadingH1(doc.body)}
      </Markdown>
      <Backlinks doc={doc} />
      <p className="page__path">{doc.path}</p>
    </article>
  );
}
