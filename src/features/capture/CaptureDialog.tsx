import type { MouseEvent } from "react";
import { CapturePanel } from "./CapturePanel";
import type { DraftStore } from "./drafts";

export interface CaptureDialogProps {
  save(text: string): Promise<void>;
  /** Closes the dialog; the draft stays. */
  hide(): void;
  drafts: DraftStore;
  /** When the dialog was opened (focuses the text again). */
  shownAt: number;
}

/** The capture slip in the page, for the web and the demo. */
export function CaptureDialog({ save, hide, drafts, shownAt }: CaptureDialogProps) {
  // A click on the dimmed page closes the dialog (the draft stays).
  const onBackdrop = (event: MouseEvent<HTMLDivElement>) => {
    if (event.target === event.currentTarget) hide();
  };
  return (
    <div className="capture-overlay" onMouseDown={onBackdrop}>
      <div className="capture-dialog" role="dialog" aria-modal="true" aria-label="记一笔">
        <CapturePanel variant="dialog" save={save} hide={hide} drafts={drafts} shownAt={shownAt} />
      </div>
    </div>
  );
}
