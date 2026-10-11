/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The same frontend ships in two ways:
//   - desktop (default mode): loaded by the Tauri webview, built into dist/
//   - web (`--mode web`): static site for GitHub Pages, built into dist-web/
const WEB_BASE = "/reading-app/";

export default defineConfig(({ mode }) => {
  const isWeb = mode === "web";

  return {
    plugins: [react()],
    base: isWeb ? WEB_BASE : "/",
    // Keep Rust compiler output visible when Vite runs under `tauri dev`.
    clearScreen: false,
    server: {
      // tauri.conf.json points devUrl at this port, so fail instead of picking another one.
      port: 1420,
      strictPort: true,
      watch: {
        // Rust sources are rebuilt by the Tauri CLI, not by Vite.
        ignored: ["**/src-tauri/**"],
      },
    },
    build: {
      outDir: isWeb ? "dist-web" : "dist",
    },
    test: {
      // Only look inside src/ so Vitest never walks src-tauri/target or build output.
      include: ["src/**/*.test.{ts,tsx}"],
    },
  };
});
