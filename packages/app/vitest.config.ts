import { defineConfig } from "vitest/config";

export default defineConfig({
  resolve: {
    alias: { "@tally/bridge/tauri": new URL("../bridge/src/tauri.ts", import.meta.url).pathname },
  },
  test: {
    include: ["test/**/*.test.ts"],
    environment: "node",
  },
});
