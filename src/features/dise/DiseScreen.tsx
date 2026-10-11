import { useEffect, useState } from "react";
import { Link, useParams } from "react-router";
import { fmString, parseDise, type DiseEntry } from "../../domain";
import { useKb } from "../../app/KbContext";
import { diseEntryRoute, safeDecode } from "../../app/links";
import { Chip, cx, EmptyState, PageHeader, Seal, SealSlot } from "../../ui/components";
import { Markdown } from "../../ui/Markdown";
import { formatIsoDate } from "../../ui/format";
import { diseToggle } from "./toggle";

const READ_ONLY_HINT = "开发预览是只读的：请在桌面 App 里盖章";

interface EntryCardProps {
  entry: DiseEntry;
  index: number;
  writable: boolean;
  pending: boolean;
  justStamped: boolean;
  error: string | null;
  onToggle(entry: DiseEntry): void;
}

function EntryCard({ entry, index, writable, pending, justStamped, error, onToggle }: EntryCardProps) {
  const { resolveLink } = useKb();
  return (
    <article
      id={`dise-${entry.id}`}
      className={cx("entry rise", entry.confirmed && "is-confirmed", entry.draft && "is-draft")}
      style={{ animationDelay: `${Math.min(index, 8) * 40}ms` }}
      aria-labelledby={`dise-${entry.id}-title`}
    >
      <div className="entry__gutter">
        <span className="entry__id">{entry.id}</span>
      </div>
      <div className="entry__main">
        <header className="entry__head">
          <h3 className="entry__title" id={`dise-${entry.id}-title`}>
            {entry.title}
          </h3>
          <span className="entry__chips">
            {entry.notes.map((note) => (
              <Chip key={note} tone="ink">
                {note}
              </Chip>
            ))}
            {entry.confirmed ? (
              <Chip tone="seal">已确认 ✅</Chip>
            ) : (
              <Chip tone="ochre" title={entry.draft ? "AI 起草，待你确认" : "还没有确认"}>
                草稿
              </Chip>
            )}
          </span>
        </header>
        {entry.quote ? (
          <blockquote className="entry__quote">
            <Markdown resolve={resolveLink}>{entry.quote}</Markdown>
          </blockquote>
        ) : null}
        {entry.bodyMarkdown ? (
          <Markdown resolve={resolveLink} className="entry__body">
            {entry.bodyMarkdown}
          </Markdown>
        ) : null}
        <footer className="entry__foot">
          {error ? (
            <p className="entry__error" role="alert">
              {error}
            </p>
          ) : null}
          <span title={writable ? undefined : READ_ONLY_HINT} className="entry__toggle-wrap">
            <button
              type="button"
              className={cx("btn", "btn--small", entry.confirmed ? "btn--quiet" : "btn--seal-outline")}
              onClick={() => onToggle(entry)}
              disabled={!writable || pending}
              aria-describedby={`dise-${entry.id}-title`}
            >
              {pending ? "写入中…" : entry.confirmed ? "取消确认" : "盖章确认"}
            </button>
          </span>
        </footer>
      </div>
      <div className="entry__seal" aria-hidden={!entry.confirmed}>
        {entry.confirmed ? <Seal animate={justStamped} /> : <SealSlot />}
      </div>
    </article>
  );
}

export function DiseScreen() {
  const { model, writable, editFile, notify, resolveLink } = useKb();
  const params = useParams();
  const focusId = safeDecode(params.entryId);
  const [pendingId, setPendingId] = useState<string | null>(null);
  const [stampedId, setStampedId] = useState<string | null>(null);
  const [error, setError] = useState<{ id: string; message: string } | null>(null);

  useEffect(() => {
    if (!focusId) return;
    const target = document.getElementById(`dise-${focusId}`);
    target?.scrollIntoView({ block: "start" });
    target?.classList.add("is-focused");
    const timer = window.setTimeout(() => target?.classList.remove("is-focused"), 1800);
    return () => window.clearTimeout(timer);
  }, [focusId]);

  const dise = model.dise;
  const path = model.disePath;
  if (!dise || !path) {
    return (
      <div className="screen">
        <PageHeader eyebrow="底色" title="我的决策底色" />
        <EmptyState title="知识库里没有 self/底色.md">底色文件建好以后，这里会变成一张人物志。</EmptyState>
      </div>
    );
  }

  const confirmedCount = dise.entries.filter((e) => e.confirmed).length;
  const version = fmString(dise.frontmatter, "版本");
  const status = fmString(dise.frontmatter, "状态");
  const updated = fmString(dise.frontmatter, "更新");

  const toggle = async (entry: DiseEntry) => {
    setPendingId(entry.id);
    setError(null);
    try {
      const { edit, message } = diseToggle(entry.id);
      const result = await editFile(path, edit, message);
      const stamped = parseDise(result.next).entries.find((e) => e.id === entry.id)?.confirmed ?? false;
      setStampedId(stamped ? entry.id : null);
      notify(stamped ? `已盖章并提交：${result.message}` : `已提交：${result.message}`);
    } catch (e) {
      setError({ id: entry.id, message: e instanceof Error ? e.message : String(e) });
    } finally {
      setPendingId(null);
    }
  };

  let index = 0;
  return (
    <div className="screen dise">
      <PageHeader
        eyebrow="人物志 · 底色"
        title={dise.title ?? "我的决策底色"}
        meta={
          <>
            {version && !(dise.title ?? "").includes(version) ? <Chip tone="ink">{version}</Chip> : null}
            {status ? <span>{status}</span> : null}
            {updated ? <span>更新于 {formatIsoDate(updated)}</span> : null}
          </>
        }
      />

      <div className="dise__summary">
        <div className="pips" aria-label={`已确认 ${confirmedCount} 条，共 ${dise.entries.length} 条`}>
          {dise.entries.map((entry) => (
            <Link
              key={entry.id}
              to={diseEntryRoute(entry.id)}
              className={cx("pip", entry.confirmed && "is-on")}
              title={`${entry.id} ${entry.title}${entry.confirmed ? " · 已确认" : ""}`}
            />
          ))}
        </div>
        <p className="dise__count">
          已确认 <span className="num">{confirmedCount}</span> / <span className="num">{dise.entries.length}</span>
          {writable ? null : <span className="dise__readonly"> · 只读预览</span>}
        </p>
      </div>

      {dise.introMarkdown ? (
        <details className="dise__intro">
          <summary>框架说明</summary>
          <Markdown resolve={resolveLink}>{dise.introMarkdown}</Markdown>
        </details>
      ) : null}

      <div className="dise__layout">
        <div className="dise__sections">
          {dise.sections.map((section) => (
            <section className="dise-section" key={section.heading || "untitled"} aria-label={section.heading}>
              <header className="dise-section__head">
                {section.ordinal ? (
                  <span className="dise-section__ordinal" aria-hidden="true">
                    {section.ordinal}
                  </span>
                ) : null}
                <h2 className="dise-section__title">{section.title || "未分组"}</h2>
                <span className="dise-section__count">
                  <span className="num">{section.entries.length}</span> 条
                </span>
              </header>
              {section.introMarkdown ? (
                <Markdown resolve={resolveLink} className="dise-section__intro">
                  {section.introMarkdown}
                </Markdown>
              ) : null}
              {section.entries.map((entry) => (
                <EntryCard
                  key={entry.id}
                  entry={entry}
                  index={index++}
                  writable={writable}
                  pending={pendingId === entry.id}
                  justStamped={stampedId === entry.id}
                  error={error?.id === entry.id ? error.message : null}
                  onToggle={(e) => void toggle(e)}
                />
              ))}
            </section>
          ))}
        </div>

        <nav className="dise__rail" aria-label="条目索引">
          <p className="card-label">索引</p>
          <ol>
            {dise.entries.map((entry) => (
              <li key={entry.id}>
                <Link to={diseEntryRoute(entry.id)} className={cx("rail__item", entry.confirmed && "is-on")}>
                  <span className="rail__id">{entry.id}</span>
                  <span className="rail__title">{entry.title}</span>
                </Link>
              </li>
            ))}
          </ol>
        </nav>
      </div>
    </div>
  );
}
