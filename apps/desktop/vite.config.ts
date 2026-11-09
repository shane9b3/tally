import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// `pnpm --filter tally-desktop dev:web` serves the UI at :5173 (Tauri's devUrl).
// In a plain browser it runs against MockClient.
// TALLY_E2E_PORT lets concurrent e2e runs use their own dev server (default 5173 = Tauri devUrl).
const port = Number(process.env.TALLY_E2E_PORT ?? 5173);

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Pre-bundle the deps that only lazy chunks import (charts, command palette, Tauri window API).
  // Otherwise Vite discovers them on first use, re-optimizes and reloads the page mid-session,
  // which wipes state in a running e2e test (and flashes a dev reload in `dev:web`).
  optimizeDeps: {
    include: [
      "@tally/app > echarts/core",
      "@tally/app > echarts/charts",
      "@tally/app > echarts/components",
      "@tally/app > echarts/renderers",
      "@tally/app > cmdk",
      "@tally/app > zustand",
      "@tauri-apps/api/core",
      "@tauri-apps/api/webviewWindow",
    ],
  },
  server: {
    port,
    strictPort: true,
    host: "localhost",
    // Transform the lazy chunks' module graph at server start instead of on first dialog open.
    warmup: { clientFiles: ["../../packages/app/src/lazyChunks.ts", "./src/main.tsx"] },
  },
  preview: { port, strictPort: true },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "safari16",
    chunkSizeWarningLimit: 1024,
    sourcemap: false,
    rollupOptions: {
      output: {
        // Stable vendor chunks: they change only when the dependency does,
        // so the (frequently changing) app chunk is the only one re-fetched.
        manualChunks(id) {
          if (!id.includes("node_modules")) return undefined;
          if (/[\\/]node_modules[\\/](\.pnpm[\\/])?(react|react-dom|scheduler)[@\\/]/.test(id)) return "vendor-react";
          if (id.includes("echarts") || id.includes("zrender")) return "vendor-echarts";
          return undefined;
        },
      },
    },
  },
});
