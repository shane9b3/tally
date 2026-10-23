import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// `pnpm --filter tally-desktop dev:web` serves the UI at :5173 (Tauri's devUrl).
// In a plain browser it runs against MockClient.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true, host: "localhost" },
  preview: { port: 5173, strictPort: true },
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
