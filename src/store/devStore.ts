import type { KbFileEntry, KbStore } from "./types";

/** Message shown when something tries to write in the read-only dev preview. */
export const DEV_READ_ONLY_MESSAGE =
  "开发预览（pnpm dev:kb）是只读的，不能写入知识库。请在桌面 App 里修改。";

type Fetch = (input: string, init?: RequestInit) => Promise<Response>;

async function failure(response: Response, what: string): Promise<Error> {
  const detail = (await response.text().catch(() => "")).trim();
  return new Error(`${what}失败（${response.status}）${detail ? `：${detail}` : ""}`);
}

/**
 * Read-only store for `pnpm dev:kb`: talks to the dev-server middleware in
 * dev/kbDevServer.ts. Writes always throw.
 */
export function createDevStore(
  options: { base?: string; fetch?: Fetch } = {},
): KbStore {
  const base = options.base ?? import.meta.env.BASE_URL;
  const doFetch: Fetch = options.fetch ?? ((input, init) => fetch(input, init));
  const prefix = `${base.endsWith("/") ? base : `${base}/`}__kb`;

  return {
    kind: "dev",
    async list(): Promise<KbFileEntry[]> {
      const response = await doFetch(`${prefix}/list`);
      if (!response.ok) throw await failure(response, "读取文件列表");
      return (await response.json()) as KbFileEntry[];
    },
    async read(path: string): Promise<string> {
      const response = await doFetch(`${prefix}/read?path=${encodeURIComponent(path)}`);
      if (!response.ok) throw await failure(response, `读取 ${path} `);
      return response.text();
    },
    async write(): Promise<void> {
      throw new Error(DEV_READ_ONLY_MESSAGE);
    },
  };
}
