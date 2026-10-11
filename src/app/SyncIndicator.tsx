import { useId } from "react";
import { cx } from "../ui/components";
import { desktop, type Desktop } from "./desktop";
import { describeSync, useSyncStatus, type SyncStatus } from "./sync";

export interface SyncIndicatorViewProps {
  status: SyncStatus | null;
  /** A sync started from here is running. */
  requesting?: boolean;
  onSync(): void;
  now?: number;
}

/**
 * The sync state at the bottom of the sidebar. A click syncs now; hovering
 * (or focusing) shows a card that says more, and what to do when a person
 * has to step in.
 */
export function SyncIndicatorView({ status, requesting = false, onSync, now }: SyncIndicatorViewProps) {
  const view = describeSync(status, now);
  const running = view.running || requesting;
  const cardId = useId();
  return (
    <div className={cx("sync", `is-${view.tone}`, running && "is-running")}>
      <button
        type="button"
        className="sync__button"
        onClick={onSync}
        disabled={running}
        aria-label={`${view.title}。点一下立即同步`}
        aria-describedby={cardId}
      >
        <span className="sync__mark" aria-hidden="true" />
        {view.count && !running ? <span className="sync__count">{view.count}</span> : null}
        <span className="sync__label">{running ? "同步中" : view.label}</span>
      </button>
      <div className="sync__card" id={cardId} role="tooltip">
        <p className="sync__card-title">{running ? "正在和远端同步…" : view.title}</p>
        {view.explain && !running ? <p className="sync__card-text">{view.explain}</p> : null}
        {view.paths.length > 0 && !running ? (
          <ul className="sync__paths">
            {view.paths.map((path) => (
              <li key={path}>
                <code>{path}</code>
              </li>
            ))}
          </ul>
        ) : null}
        {running ? null : <p className="sync__card-hint">点一下立即同步</p>}
      </div>
    </div>
  );
}

/** The live indicator (desktop only: the caller decides whether to show it). */
export function SyncIndicator({ bridge = desktop }: { bridge?: Desktop }) {
  const { status, syncNow, requesting } = useSyncStatus(bridge, true);
  return <SyncIndicatorView status={status} requesting={requesting} onSync={() => void syncNow()} />;
}
