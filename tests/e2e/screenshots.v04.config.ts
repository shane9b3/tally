/** Config for the v0.4 screenshot script (screenshots.v04.ts); see that file. */
import base from "./playwright.config";
import { defineConfig } from "@playwright/test";

export default defineConfig({ ...base, testMatch: "screenshots.v04.ts", use: { ...base.use, deviceScaleFactor: 2 } });
