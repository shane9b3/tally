import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// `pnpm --filter tally-desktop dev:web` serves the UI at :5173 (Tauri's devUrl).
// In a plain browser it runs against MockClient.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  resolve: {
    alias: {
      // TODO(bridge): replace with a `./tauri` export in packages/bridge/package.json.
      "@tally/bridge/tauri": decodeURIComponent(new URL("../../packages/bridge/src/tauri.ts", import.meta.url).pathname),
    },
  },
  server: { port: 5173, strictPort: true, host: "localhost" },
  preview: { port: 5173, strictPort: true },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "safari16",
    chunkSizeWarningLimit: 1024,
    sourcemap: false,
  },
});
