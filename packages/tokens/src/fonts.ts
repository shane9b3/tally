/**
 * Side-effect module that bundles the Inter Variable font (via
 * @fontsource-variable/inter). Import once from the app entry point:
 *
 *   import "@tally/tokens/fonts";
 *   import "@tally/tokens/tokens.css";
 *
 * The grid must `await document.fonts.load('13px "Inter Variable"')` before
 * its first paint.
 */
import "@fontsource-variable/inter";
