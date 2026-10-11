import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { mkdtempSync, mkdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { handleKbRequest, isLocalHostHeader, isLoopback, kbDevServer } from "./kbDevServer";

let root: string;

beforeAll(() => {
  root = realpathSync(mkdtempSync(path.join(tmpdir(), "dise-kbdev-")));
  mkdirSync(path.join(root, "self"));
  writeFileSync(path.join(root, "self", "底色.md"), "# 底色\n");
});

afterAll(() => {
  rmSync(root, { recursive: true, force: true });
});

const local = { remoteAddress: "127.0.0.1", host: "127.0.0.1:1420" };

describe("isLoopback / isLocalHostHeader", () => {
  it("accepts only loopback peers", () => {
    expect(isLoopback("127.0.0.1")).toBe(true);
    expect(isLoopback("::1")).toBe(true);
    expect(isLoopback("::ffff:127.0.0.1")).toBe(true);
    expect(isLoopback("192.168.1.20")).toBe(false);
    expect(isLoopback(undefined)).toBe(false);
  });

  it("accepts only local host names (DNS-rebinding guard)", () => {
    expect(isLocalHostHeader("localhost:1420")).toBe(true);
    expect(isLocalHostHeader("127.0.0.1:1420")).toBe(true);
    expect(isLocalHostHeader("[::1]:1420")).toBe(true);
    expect(isLocalHostHeader("localhost")).toBe(true);
    expect(isLocalHostHeader("evil.example:1420")).toBe(false);
    expect(isLocalHostHeader(undefined)).toBe(false);
  });
});

describe("handleKbRequest", () => {
  it("lists markdown files as JSON", async () => {
    const res = await handleKbRequest(root, { method: "GET", url: "/list", ...local });
    expect(res.status).toBe(200);
    expect(res.headers["Content-Type"]).toMatch(/application\/json/);
    expect(res.headers["Cache-Control"]).toBe("no-store");
    expect(JSON.parse(res.body).map((f: { path: string }) => f.path)).toEqual(["self/底色.md"]);
  });

  it("reads one file by its encoded path", async () => {
    const url = `/read?path=${encodeURIComponent("self/底色.md")}`;
    const res = await handleKbRequest(root, { method: "GET", url, ...local });
    expect(res.status).toBe(200);
    expect(res.headers["Content-Type"]).toMatch(/text\/markdown/);
    expect(res.body).toBe("# 底色\n");
  });

  it("rejects unsafe paths, missing files and missing parameters", async () => {
    const get = (url: string) => handleKbRequest(root, { method: "GET", url, ...local });
    expect((await get(`/read?path=${encodeURIComponent("../x.md")}`)).status).toBe(400);
    expect((await get("/read?path=self%2F%E6%B2%A1%E6%9C%89.md")).status).toBe(404);
    expect((await get("/read")).status).toBe(400);
    expect((await get("/nope")).status).toBe(404);
  });

  it("is read-only", async () => {
    for (const method of ["POST", "PUT", "DELETE", "PATCH"]) {
      const res = await handleKbRequest(root, { method, url: "/list", ...local });
      expect(res.status).toBe(405);
    }
  });

  it("refuses non-local peers and foreign host names", async () => {
    const remote = await handleKbRequest(root, {
      method: "GET",
      url: "/list",
      remoteAddress: "10.0.0.8",
      host: "127.0.0.1:1420",
    });
    expect(remote.status).toBe(403);
    const rebinding = await handleKbRequest(root, {
      method: "GET",
      url: "/list",
      remoteAddress: "127.0.0.1",
      host: "attacker.example:1420",
    });
    expect(rebinding.status).toBe(403);
  });

  it("reports a missing KB folder clearly", async () => {
    const res = await handleKbRequest(path.join(root, "missing"), {
      method: "GET",
      url: "/list",
      ...local,
    });
    expect(res.status).toBe(500);
    expect(res.body).toMatch(/知识库/);
  });
});

describe("kbDevServer", () => {
  it("only applies to the dev server", () => {
    const plugin = kbDevServer({ root });
    expect(plugin.apply).toBe("serve");
    expect(plugin.name).toBe("dise-kb-dev-server");
  });
});
