import { useEffect, useRef, type ReactNode } from "react";
import { Link, useLocation } from "react-router";
import { useKb } from "./KbContext";
import { NAV_ITEMS, navSectionForGroup, safeDecode, type NavSection } from "./links";
import { cx } from "../ui/components";

/** Which nav item is active for the current route. */
function useActiveSection(): NavSection {
  const { pathname } = useLocation();
  const { model } = useKb();
  if (pathname.startsWith("/dise")) return "dise";
  if (pathname.startsWith("/rulings")) return "rulings";
  if (pathname.startsWith("/knowledge")) return "knowledge";
  if (pathname.startsWith("/raw")) return "raw";
  if (pathname.startsWith("/page/")) {
    const name = safeDecode(pathname.slice("/page/".length));
    const doc = model.byName.get(name);
    return doc ? navSectionForGroup(doc.group) : "knowledge";
  }
  return "today";
}

/** App frame: the book-spine navigation on the left, the desk on the right. */
export function Shell({ children }: { children: ReactNode }) {
  const { model, reload, reloading, notice, sourceLabel, sourceDetail, writable } = useKb();
  const active = useActiveSection();
  const location = useLocation();
  const desk = useRef<HTMLElement>(null);

  // The desk is the scroll container: start every screen at the top. Entry
  // routes such as /dise/D05 scroll to their entry themselves (child effects
  // run first), so they are left alone.
  useEffect(() => {
    if (/^\/dise\/./.test(location.pathname)) return;
    desk.current?.scrollTo({ top: 0 });
  }, [location.pathname, location.search]);

  const attention = {
    rulings: model.dashboard.pendingRulings.high > 0 || model.dashboard.uncheckedAudit > 0,
    raw: model.dashboard.inboxCaptures > 0,
  } as Partial<Record<NavSection, boolean>>;

  return (
    <div className="shell">
      <nav className="spine" aria-label="主导航">
        <Link to="/" className="spine__brand" aria-label="底色 · 今日">
          底色
        </Link>
        <ul className="spine__nav">
          {NAV_ITEMS.map((item) => (
            <li key={item.section}>
              <Link
                to={item.to}
                className={cx("spine__link", active === item.section && "is-active")}
                aria-current={active === item.section ? "page" : undefined}
              >
                <span className="spine__label">{item.label}</span>
                {attention[item.section] ? <span className="spine__dot" aria-label="有待处理" /> : null}
              </Link>
            </li>
          ))}
        </ul>
        <div className="spine__foot">
          <button
            type="button"
            className={cx("spine__reload", reloading && "is-spinning")}
            onClick={() => void reload()}
            title="重新载入知识库"
            aria-label="重新载入知识库"
            disabled={reloading}
          >
            ↻
          </button>
          <span
            className={cx("spine__source", !writable && "is-readonly")}
            title={sourceDetail ? `${sourceLabel}：${sourceDetail}` : sourceLabel}
          >
            {sourceLabel}
          </span>
        </div>
      </nav>
      <main className="desk" ref={desk} id="desk">
        {children}
      </main>
      <div className={cx("toast", notice && "is-visible")} role="status" aria-live="polite">
        {notice}
      </div>
    </div>
  );
}
