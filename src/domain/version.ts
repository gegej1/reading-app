/** Display name of the app. Single source of truth for UI copy that names the app. */
export const APP_DISPLAY_NAME = "底色";

/**
 * Formats the label used wherever the app identifies itself together with a
 * version, e.g. "底色 v0.1.0".
 *
 * A leading "v" and surrounding whitespace are tolerated. An empty version
 * falls back to the bare display name.
 */
export function formatVersionLabel(version: string): string {
  const normalized = version.trim().replace(/^v/i, "");
  return normalized === ""
    ? APP_DISPLAY_NAME
    : `${APP_DISPLAY_NAME} v${normalized}`;
}
