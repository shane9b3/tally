import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// `pnpm --filter @tally/grid dev` serves the demo (demo/index.html).
export default defineConfig({
  root: "demo",
  plugins: [react()],
  server: { port: 5174, open: true },
  build: { outDir: "../dist", emptyOutDir: true, chunkSizeWarningLimit: 1024 },
});
