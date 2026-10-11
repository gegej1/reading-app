import { parseDise, toggleConfirmed } from "../../domain";

/** Commit message for a confirm toggle, e.g. `底色：确认 D05`. */
export function confirmMessage(id: string, confirmed: boolean): string {
  return `底色：${confirmed ? "确认" : "取消确认"} ${id}`;
}

/**
 * The edit behind the 盖章确认 / 取消确认 button: flips the ✅ on one heading,
 * and names the commit after the state the file ends up in.
 */
export function diseToggle(id: string) {
  return {
    edit: (current: string) => toggleConfirmed(current, id),
    message: (next: string) =>
      confirmMessage(id, parseDise(next).entries.find((entry) => entry.id === id)?.confirmed ?? false),
  };
}
