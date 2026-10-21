/** Config for the v0.5 screenshot script (screenshots.v05.ts); see that file. */
import base from "./playwright.config";
import { defineConfig } from "@playwright/test";

export default defineConfig({ ...base, testMatch: "screenshots.v05.ts", use: { ...base.use, deviceScaleFactor: 2 } });
