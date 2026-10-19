import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    // Let `?raw` imports of tokens.css return the real text (Vitest blanks
    // CSS by default).
    css: { include: [/tokens\.css/] },
  },
});
