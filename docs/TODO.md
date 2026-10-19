# Tally TODO (as of 2026-10-19, commit 3655744)

Status: waves 1–4 complete; wave 5 planned, NOT started (owner to say when).

## Done
- All 408 functions (405 AOO 4.1 + IFERROR, IFNA, NUMBERVALUE, ISOWEEKNUM); 5,003 conformance cases. `cargo run -p xtask -- function-coverage`.
- Engine v0.3: names, CSE arrays, merges, freeze, sort, AutoFilter, find/replace, borders, paste special, fill, move, conditional formatting, validation, notes, links, protection, page setup, document properties, locales (en-US, en-GB, de-DE, fr-FR, es-ES, ja-JP), goal seek.
- Files: ODS, XLSX, CSV read/write with hardened parsers (docs/security.md).
- Desktop UI for all of the above, light and dark themes (docs/qa/screenshots/).
- Security + correctness reviews, 2 rounds each, all findings fixed (docs/reviews/).
- Gates: 1,000 Rust tests, 598 TS tests, 32 e2e; clippy and fmt clean.

## Next: wave 5 (docs/wave5-plan.md)
1. W5-0a contracts (engine lead) — DTOs, Op variants, model fields, stubs. Blocks everything else.
2. W5-0b shell seams (UI) — Tauri commands_v4.rs, TauriClient/MockClient stubs, menu items.
3. In parallel: W5-1 charts (engine+IO), W5-2 DataPilot/outline/data tools, W5-3 print/PDF (krilla) + iterative calc + precision-as-shown + SXC/XLS/HTML/dBase, W5-4 function hooks + wizard metadata + tally-l10n, W5-5 UI charts/print, W5-6 UI data tools/dialogs/a11y/i18n.
4. Review round 3 (security + correctness), fixes, release build, final report.

## Known gaps / follow-ups
- ODS output not yet verified in a real OpenOffice (headless hangs while the GUI copy runs; plan §5 proposes an isolated-profile Basic-macro oracle).
- Release bundle last built at wave 2; rebuild: `pnpm --filter tally-desktop tauri build`.
- Tauri shell uses one global lock (a slow call blocks all documents); per-document locks planned.
- `bigattr.ods` (400 MiB XML attribute) still peaks ~800 MB RSS.
- Windows branch of `open_url` untested; Intel macOS / Windows / Linux builds need CI (local toolchain is arm64-only).
- Owner-only: signing certificates, notarization, updater keys, CI runners/secrets, App Store decision, fidelity corpus, AOO oracle run.

## How to run
- `pnpm install`, `cargo test --workspace --features tally-engine/io`, `pnpm -r test`, `pnpm e2e`
- `pnpm tauri dev` (desktop) or `pnpm --filter tally-desktop dev:web` (browser with mock engine, `?template=invoices`)
