// Synthetic knowledge base used by tests and by the in-memory demo store.
// Every file under ./kb mirrors the real KB formats with invented content.
// Never copy real knowledge-base content in here: this repository is public.

const modules = import.meta.glob("./kb/**/*.md", {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

/** Repo-relative POSIX path -> file content. */
export const fixtureKb: Readonly<Record<string, string>> = Object.freeze(
  Object.fromEntries(
    Object.entries(modules).map(([key, content]) => [
      key.replace(/^\.\/kb\//, ""),
      content,
    ]),
  ),
);

/** Returns one fixture file or throws, so typos in tests fail loudly. */
export function fixture(path: string): string {
  const content = fixtureKb[path];
  if (content === undefined) {
    throw new Error(`Unknown fixture: ${path}`);
  }
  return content;
}
