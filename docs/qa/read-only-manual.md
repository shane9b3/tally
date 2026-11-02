# Manual QA: read-only mode and OpenOffice lock files

The engine, shell and UI paths are covered automatically
(`crates/tally-engine/tests/read_only.rs`,
`apps/desktop/src-tauri/src/doclock.rs` and `tests.rs`,
`packages/app/test/readOnly.test.ts`, `tests/e2e/read-only.spec.ts`). These
checks need the native app **and OpenOffice side by side** and run against
the release build (`pnpm --filter tally-desktop tauri build`, then open
`Tally.app`).

Use copies in a scratch folder (for example `~/Desktop/lock-test/`): `A.ods`
and `B.ods`, each saved once. Do not use real working files. In Finder, show
hidden files (⇧⌘.) to see `.~lock.A.ods#`.

What to expect from OpenOffice: when Tally holds the lock and OpenOffice runs
under **the same macOS account**, OpenOffice says the document is "locked for
editing by yourself on a different system since <date>" (Open Read-Only /
Open / Cancel), because Tally's lock names Tally's own settings folder as its
"installation". From **another account or another Mac** (a shared folder),
OpenOffice says "locked for editing by: <your full name> ( <date> )" (Open
Read-Only / Open Copy / Cancel). Tally asks the same questions about
OpenOffice's locks.

| # | Step | Expected |
|---|---|---|
| 1 | In OpenOffice, open `A.ods` (do not close it). In Finder, look at the folder. | `.~lock.A.ods#` exists. |
| 2 | In Tally, File ▸ Open… `A.ods`. | "Document in Use": "Document file 'A.ods' is locked for editing by yourself on a different system since <date>", buttons **Open Read-Only** · Open · Cancel. (From another account: "…locked for editing by: <name> ( <date> )" with Open Read-Only · Open Copy · Cancel.) |
| 3 | Choose Open Read-Only. | `A.ods` opens; the info bar says "This document is open in read-only mode." with Edit Document; the window title and the sidebar entry read "A.ods (read-only)"; typing in a cell shows "Document opened in read-only mode." and changes nothing; Save is greyed out; Copy, Find, zoom, Print and Export as PDF work. |
| 4 | Click Edit Document (or Edit ▸ Edit Mode, ⇧⌘M). | "Document in Use" again (OpenOffice still has it): Open · Cancel for your own lock (Open Copy · Cancel for someone else's). Cancel: still read-only. |
| 5 | In Finder, open `.~lock.A.ods#` with TextEdit (read only, do not save). | OpenOffice's lock is unchanged (Tally never touches it while you only read). Close TextEdit. |
| 6 | Close `A.ods` in Tally (⌘W). | No save question; `.~lock.A.ods#` is still there (it is OpenOffice's). |
| 7 | Quit OpenOffice's `A.ods` window. | `.~lock.A.ods#` disappears. |
| 8 | In Tally, open `B.ods` normally and leave it open. Look at the folder. | `.~lock.B.ods#` exists. Open it in TextEdit: one line `<your full name>,<account>,<host>,<DD.MM.YYYY HH:MM>,file:///Users/<you>/Library/Application%20Support/<Tally's id>;` (no newline). Close TextEdit without saving. |
| 9 | In OpenOffice, File ▸ Open `B.ods`. | OpenOffice shows its "Document in Use" dialog: "locked for editing by yourself on a different system since <date>" (same account) or "…by: <your full name> ( <date> )" (another account). Choose Cancel in OpenOffice. |
| 10 | In Tally, File ▸ Save As… `B.ods` as `C.ods`. | `.~lock.B.ods#` disappears, `.~lock.C.ods#` appears. |
| 11 | In Tally, ⌘Q (no unsaved changes). | Tally quits; `.~lock.C.ods#` is gone. |
| 12 | Relaunch Tally, open `C.ods`, then force-quit Tally (Activity Monitor ▸ Force Quit). | `.~lock.C.ods#` stays (a crash leaves the lock, as with OpenOffice). |
| 13 | Relaunch Tally and open `C.ods`. | It opens for editing without a question (Tally's own stale lock is taken over). Close it: the lock goes. |
| 14 | In Finder, select `A.ods` ▸ Get Info ▸ tick **Locked**. Open it in Tally. | Opens read-only (info bar, title "(read-only)"), no lock file is written. Edit Document asks "This document cannot be edited, possibly due to missing access rights. Do you want to edit a copy of the document?": No keeps it read-only; Yes makes an editable "Untitled" copy (Save asks where). Untick Locked afterwards. |
| 15 | Open `B.ods`, type a value, then Edit ▸ Edit Mode. | "Do you want to save the changes…?" Cancel: nothing changes. Don't Save: the value is gone and the document is read-only. (Save: saved, then read-only.) |
| 16 | In OpenOffice, open `A.ods`, File ▸ Properties ▸ Security ▸ tick "Open file read-only", save and close. Open it in Tally. | Opens read-only (info bar). Edit Document switches at once (no question; Tally holds the lock). Save it in Tally, reopen in OpenOffice: still opens read-only (the flag round-trips). |
| 17 | File ▸ Open Read-Only… `B.ods`. | Opens read-only; no `.~lock.B.ods#` is written. |
| 18 | Put `D.ods` in a folder you cannot write to (e.g. a read-only disk image) and open it. | Opens; no lock file, no error. (If the file itself is read-only it opens read-only, as in step 14.) |
| 19 | Switch Tools ▸ Options ▸ Language to Deutsch and repeat step 2. | "Dokument in Verwendung" with Schreibgeschützt öffnen · Öffnen · Abbrechen; title suffix "(schreibgeschützt)"; Bearbeiten ▸ Bearbeitungsmodus. |
