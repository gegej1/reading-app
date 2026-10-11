import type { ReactNode } from "react";

export function cx(...names: (string | false | null | undefined)[]): string {
  return names.filter(Boolean).join(" ");
}

export interface PageHeaderProps {
  eyebrow?: ReactNode;
  title: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
}

/** Screen header: small label, large serif title, optional meta line and actions. */
export function PageHeader({ eyebrow, title, meta, actions }: PageHeaderProps) {
  return (
    <header className="page-head">
      <div className="page-head__text">
        {eyebrow ? <p className="page-head__eyebrow">{eyebrow}</p> : null}
        <h1 className="page-head__title">{title}</h1>
        {meta ? <div className="page-head__meta">{meta}</div> : null}
      </div>
      {actions ? <div className="page-head__actions">{actions}</div> : null}
    </header>
  );
}

export type ChipTone = "plain" | "seal" | "ochre" | "jade" | "ink";

export function Chip({ tone = "plain", children, title }: { tone?: ChipTone; children: ReactNode; title?: string }) {
  return (
    <span className={cx("chip", `chip--${tone}`)} title={title}>
      {children}
    </span>
  );
}

/** The vermilion seal 本人确认, stamped on confirmed principles. */
export function Seal({ size = "md", animate = false }: { size?: "sm" | "md"; animate?: boolean }) {
  return (
    <span className={cx("seal", `seal--${size}`, animate && "seal--stamp")} role="img" aria-label="已确认">
      <span className="seal__text" aria-hidden="true">
        本人确认
      </span>
    </span>
  );
}

/** The empty dashed square where the seal will go. */
export function SealSlot() {
  return (
    <span className="seal-slot" role="img" aria-label="未确认">
      <span className="seal-slot__text" aria-hidden="true">
        待印
      </span>
    </span>
  );
}

/**
 * Wraps a (usually disabled) control and shows `text` on hover and focus.
 * Disabled buttons do not get pointer events, so the wrapper carries the hint.
 */
export function Hint({
  text,
  children,
  align = "center",
}: {
  text: string;
  children: ReactNode;
  /** `end` anchors the bubble to the right edge (for controls near the window edge). */
  align?: "center" | "end";
}) {
  return (
    <span className={cx("hint", align === "end" && "hint--end")} data-hint={text} tabIndex={0} aria-label={text}>
      {children}
    </span>
  );
}

/** Renders standalone numbers in serif numerals; ids such as Q-008 or M4.3 stay as they are. */
export function Numbered({ text }: { text: string }) {
  const parts = text.split(/([A-Za-z]+-?\d+(?:\.\d+)*|\d+(?:\.\d+)?)/);
  return (
    <>
      {parts.map((part, index) =>
        index % 2 === 1 && /^\d/.test(part) ? (
          <span key={index} className="num">
            {part}
          </span>
        ) : (
          part
        ),
      )}
    </>
  );
}

/** Highlights query terms inside plain text. */
export function Highlight({ text, terms }: { text: string; terms: readonly string[] }) {
  const usable = terms.filter(Boolean);
  if (usable.length === 0) return <>{text}</>;
  const escaped = usable.map((t) => t.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const pattern = new RegExp(`(${escaped.join("|")})`, "gi");
  const parts = text.split(pattern);
  return (
    <>
      {parts.map((part, index) =>
        index % 2 === 1 ? (
          <mark key={index} className="hl">
            {part}
          </mark>
        ) : (
          part
        ),
      )}
    </>
  );
}

export function EmptyState({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="empty">
      <p className="empty__title">{title}</p>
      {children ? <div className="empty__text">{children}</div> : null}
    </div>
  );
}
