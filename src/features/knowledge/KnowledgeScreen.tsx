import { useMemo } from "react";
import { Link, useSearchParams } from "react-router";
import {
  fmString,
  oneLiner,
  queryTerms,
  searchDocs,
  type KbDoc,
  type KbModel,
} from "../../domain";
import { useKb } from "../../app/KbContext";
import { docRoute, groupLabel } from "../../app/links";
import { Chip, EmptyState, Highlight, PageHeader } from "../../ui/components";

interface CatalogItem {
  doc: KbDoc;
  /** Small label before the title, e.g. P-001. */
  badge: string | null;
  title: string;
  summary: string | null;
  status: string | null;
}

interface CatalogGroup {
  name: string;
  items: CatalogItem[];
}

/** Pinyin order reads naturally in a Chinese catalogue. */
const zhCollator = new Intl.Collator("zh-Hans-CN", { numeric: true });
const byTitle = (a: CatalogItem, b: CatalogItem) => zhCollator.compare(a.title, b.title);

function buildCatalog(model: KbModel): CatalogGroup[] {
  const groups: CatalogGroup[] = [];
  const precedents = model.precedents
    .map((p): CatalogItem | null => {
      const doc = model.byPath.get(p.path);
      if (!doc) return null;
      const related = p.related.length > 0 ? `关联 ${p.related.join("、")}` : null;
      return { doc, badge: p.id, title: p.title, summary: related, status: p.status };
    })
    .filter((item): item is CatalogItem => item !== null);
  if (precedents.length > 0) groups.push({ name: "判例", items: precedents });

  for (const group of model.wikiGroups) {
    groups.push({
      name: group.name,
      items: group.docs
        .map((doc) => {
          const evidence = fmString(doc.frontmatter, "证据");
          return {
            doc,
            badge: null,
            title: doc.title,
            summary: oneLiner(doc.body),
            status: evidence ? `证据 ${evidence}` : null,
          };
        })
        .sort(byTitle),
    });
  }

  if (model.systems.length > 0) {
    groups.push({
      name: "体系",
      items: model.systems
        .map((doc) => ({
          doc,
          badge: null,
          title: doc.title,
          summary: oneLiner(doc.body),
          status: fmString(doc.frontmatter, "状态"),
        }))
        .sort(byTitle),
    });
  }
  return groups;
}

function SearchResults({ query }: { query: string }) {
  const { model } = useKb();
  const hits = useMemo(() => searchDocs(model.docs, query), [model, query]);
  const terms = queryTerms(query);
  if (hits.length === 0) {
    return <EmptyState title={`没有找到「${query}」`}>试试更短的词，或者换一个说法。</EmptyState>;
  }
  return (
    <section aria-label="搜索结果">
      <p className="search__count">
        找到 <span className="num">{hits.length}</span> 页
      </p>
      <ul className="search-results">
        {hits.map((hit) => (
          <li key={hit.doc.path}>
            <Link to={docRoute(hit.doc)} className="search-hit">
              <span className="search-hit__head">
                <span className="search-hit__title">
                  <Highlight text={hit.doc.title} terms={terms} />
                </span>
                <Chip>{groupLabel(hit.doc)}</Chip>
              </span>
              {hit.snippet ? (
                <span className="search-hit__snippet">
                  <Highlight text={hit.snippet} terms={terms} />
                </span>
              ) : null}
              <span className="search-hit__path">{hit.doc.path}</span>
            </Link>
          </li>
        ))}
      </ul>
    </section>
  );
}

function Catalog({ groups }: { groups: CatalogGroup[] }) {
  if (groups.length === 0) {
    return <EmptyState title="知识层还是空的">入库以后，概念、人物和判例会出现在这里。</EmptyState>;
  }
  return (
    <div className="catalog">
      {groups.map((group, groupIndex) => (
        <section
          key={group.name}
          className="catalog__group rise"
          style={{ animationDelay: `${groupIndex * 60}ms` }}
          aria-label={group.name}
        >
          <h2 className="catalog__title">
            {group.name}
            <span className="catalog__count num">{group.items.length}</span>
          </h2>
          <ul className="catalog__list">
            {group.items.map((item) => (
              <li key={item.doc.path}>
                <Link to={docRoute(item.doc)} className="catalog-item">
                  <span className="catalog-item__head">
                    {item.badge ? <span className="catalog-item__badge">{item.badge}</span> : null}
                    <span className="catalog-item__title">{item.title}</span>
                  </span>
                  {item.summary ? <span className="catalog-item__summary">{item.summary}</span> : null}
                  {item.status ? <span className="catalog-item__status">{item.status}</span> : null}
                </Link>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </div>
  );
}

export function KnowledgeScreen() {
  const { model } = useKb();
  const [params, setParams] = useSearchParams();
  const query = params.get("q") ?? "";
  const groups = useMemo(() => buildCatalog(model), [model]);
  const total = groups.reduce((sum, g) => sum + g.items.length, 0);

  return (
    <div className="screen knowledge">
      <PageHeader
        eyebrow="知识"
        title="书架"
        meta={
          <span>
            判例、概念、人物与公司、体系，共 <span className="num">{total}</span> 页
          </span>
        }
      />
      <div className="search">
        <label className="search__label" htmlFor="kb-search">
          搜索
        </label>
        <input
          id="kb-search"
          className="search__input"
          type="search"
          placeholder="搜索标题和全文，空格分隔多个词"
          value={query}
          autoComplete="off"
          spellCheck={false}
          onChange={(event) => {
            const value = event.target.value;
            setParams(value ? { q: value } : {}, { replace: true });
          }}
        />
      </div>
      {query.trim() ? <SearchResults query={query} /> : <Catalog groups={groups} />}
    </div>
  );
}
