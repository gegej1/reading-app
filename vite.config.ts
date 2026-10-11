/// <reference types="vitest/config" />
import { fileURLToPath } from "node:url";
import { defineConfig, type PluginOption } from "vite";
import react from "@vitejs/plugin-react";

// The same frontend ships in two ways:
//   - desktop (default mode): loaded by the Tauri webview, built into dist/
//   - web (`--mode web`): static site for GitHub Pages, built into dist-web/
// The desktop build has a second page, capture.html: the floating capture
// window (记一笔). The web build opens the same slip as a dialog instead.
// Plus a development-only mode:
//   - kb (`pnpm dev:kb`): dev server that serves the local knowledge base
//     read-only at /__kb (see dev/kbDevServer.ts). Never used by a build.
const WEB_BASE = "/reading-app/";

const page = (name: string) => fileURLToPath(new URL(name, import.meta.url));

export default defineConfig(async ({ command, mode }) => {
  const isWeb = mode === "web";
  const isKbDev = mode === "kb" && command === "serve";

  const plugins: PluginOption[] = [react()];
  if (isKbDev) {
    // Loaded lazily so the middleware code is only evaluated for `dev:kb`.
    const { kbDevServer } = await import("./dev/kbDevServer");
    plugins.push(kbDevServer());
  }

  return {
    plugins,
    base: isWeb ? WEB_BASE : "/",
    // Keep Rust compiler output visible when Vite runs under `tauri dev`.
    clearScreen: false,
    server: {
      // tauri.conf.json points devUrl at this port, so fail instead of picking another one.
      port: 1420,
      strictPort: true,
      // The kb preview exposes personal files: bind to the loopback address only.
      host: isKbDev ? "127.0.0.1" : undefined,
      watch: {
        // Rust sources are rebuilt by the Tauri CLI, not by Vite.
        ignored: ["**/src-tauri/**"],
      },
    },
    build: {
      outDir: isWeb ? "dist-web" : "dist",
      rollupOptions: {
        input: {
          main: page("index.html"),
          ...(isWeb ? {} : { capture: page("capture.html") }),
        },
      },
    },
    test: {
      // Only look inside src/ and dev/ so Vitest never walks src-tauri/target or build output.
      include: ["src/**/*.test.{ts,tsx}", "dev/**/*.test.ts"],
    },
  };
});
