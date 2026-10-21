/**
 * `@tally/app/dev`: the browser demo/e2e engine. Loaded only by dev builds
 * (`import.meta.env.DEV`) through a dynamic `import()`, so the production
 * desktop bundle never contains it (security review 3, MEDIUM-7;
 * `apps/desktop/scripts/check-bundle.sh` verifies the built output).
 */
export { WebMockClient } from "./webMock";
