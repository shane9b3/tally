/** Config for the v0.3 screenshot script (screenshots.v03.ts); see that file. */
import base from "./playwright.config";
import { defineConfig } from "@playwright/test";

export default defineConfig({ ...base, testMatch: "screenshots.v03.ts", use: { ...base.use, deviceScaleFactor: 2 } });
