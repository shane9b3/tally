# Keyboard shortcuts

Generated from `packages/app/src/keymap.ts` (`BINDINGS`, `GRID_KEYS`) and `packages/app/src/options/shortcutsDoc.ts`; `packages/app/test/keymap.test.ts` fails when this file is out of date. Regenerate with `TALLY_WRITE_SHORTCUTS=1 pnpm --filter @tally/app test -- keymap`.

⌘ is Ctrl on Windows and Linux; ⌃ is the Control key on every platform.

## Commands (keymap table)

| Keys | Command | Id |
|---|---|---|
| ⌘Z | Undo | `edit.undo` |
| ⇧⌘Z / ⌘Y | Redo | `edit.redo` |
| ⌘B | Bold | `format.bold` |
| ⌘I | Italic | `format.italic` |
| ⌘U | Underline | `format.underline` |
| ⇧⌘L | Align left | `format.alignLeft` |
| ⇧⌘E | Align centre | `format.alignCenter` |
| ⇧⌘R | Align right | `format.alignRight` |
| ⇧⌘1 | Number format: Number | `format.number.number` |
| ⇧⌘2 | Number format: Scientific | `format.number.scientific` |
| ⇧⌘3 | Number format: Date | `format.number.date` |
| ⇧⌘4 | Format as currency | `format.currency` |
| ⇧⌘5 | Format as percent | `format.percent` |
| ⇧⌘6 | Number format: General | `format.number.general` |
| ⇧⌘G | At a glance | `view.glance` |
| ⌘/ | Keyboard shortcuts | `help.shortcuts` |
| ⌘K | Command palette | `palette.toggle` |
| ⌘\ | Toggle sidebar | `view.toggleSidebar` |
| ⌘S | Save | `file.save` |
| ⇧⌘S | Save as… | `file.saveAs` |
| ⌘N | New workbook | `file.new` |
| ⌘O | Open… | `file.open` |
| ⌘= / ⌘+ | Zoom in | `view.zoomIn` |
| ⌘− | Zoom out | `view.zoomOut` |
| ⌘0 | Actual size | `view.zoomReset` |
| ⌘F | Find… | `edit.find` |
| ⌥⌘F | Find & replace… | `edit.findReplace` |
| ⌘G | Find next | `edit.findNext` |
| ⌥⌘V | Paste special… | `edit.pasteSpecial` |
| ⌘D | Fill down | `edit.fillDown` |
| ⌘R | Fill right | `edit.fillRight` |
| ⇧⌘F9 / F9 | Recalculate all | `data.recalc` |
| ⌘F3 | Define name… | `data.defineName` |
| Esc | Cancel copy | `edit.cancelClip` |
| ⌃PgDn | Next sheet | `sheet.next` |
| ⌃PgUp | Previous sheet | `sheet.prev` |
| ⇧F2 / ⌥⌘C | Insert / edit note | `insert.note` |
| ⌥⌘K | Hyperlink… | `insert.hyperlink` |
| ⌘F2 | Function… | `insert.function` |
| ⌘1 | Format cells… | `format.cells` |
| F12 | Group | `data.group` |
| ⌘F12 | Ungroup | `data.ungroup` |
| F5 | Navigator | `view.navigator` |
| ⇧F7 | Trace precedents | `tools.tracePrecedents` |
| ⇧F5 | Trace dependents | `tools.traceDependents` |
| ⌘F8 | Value highlighting | `view.valueHighlighting` |
| ⌘F11 | Styles panel | `view.stylesPanel` |
| ⌃` | Show formulas | `view.showFormulas` |
| ⌘P | Print… | `file.print` |
| ⌘, | Options… | `tools.options` |
| ⇧⌘Y | Repeat last command | `edit.repeat` |
| ⇧⌘J | Full screen | `view.fullScreen` |
| ⇧⌘T | Go to the Name Box | `edit.focusNameBox` |
| ⇧F11 | Save as template… | `file.saveAsTemplate` |
| ⌘M | Clear formats | `edit.clearFormats` |

## Grid and editor keys

### Move

| Keys | Action |
|---|---|
| ← ↑ → ↓ | Move (⇧ extends the selection) |
| ⌘ + arrows | Jump to the edge of the data |
| Home / End | Start / end of the row |
| ⌘Home / ⌘End | A1 / last used cell |
| PgUp / PgDn | One screen up / down |
| Tab / ⇧Tab | Next / previous cell |

### Select

| Keys | Action |
|---|---|
| ⌘A | Select the data, then everything |
| ⌘ + click | Add a range to the selection |
| F8 / ⇧F8 | Extend-selection mode / add-selection mode (toggle) |
| ⌘* (numpad) / ⇧⌘8 | Select the data area |
| ⌘/ (numpad) | Select the array formula around the cursor |

### Edit

| Keys | Action |
|---|---|
| F2 | Edit the active cell |
| Enter / ⇧Enter | Commit and move down / up |
| ⌥Enter | New line in the cell |
| Esc | Cancel editing |
| ⌫ / Delete | Clear contents |
| ⌘C / ⌘X / ⌘V | Copy / cut / paste |
| ⇧⌘V | Paste values only |
| ⌥⌘V | Paste special… |
| ⌘D / ⌘R | Fill down / right |
| ⌘X then ⌘V | Move cells (references follow) |
| Delete | Delete Contents dialog (⌫ clears at once; Tools ▸ Options ▸ Input turns the dialog off) |
| ⌥↩ | Fill the selection with the entered text |
| ⌃D | Selection list (entries of the column) |

### Formulas

| Keys | Action |
|---|---|
| ⌃⇧↩ | Enter as an array formula over the selection |
| Arrows after = + ( , | Point at a reference |
| Tab / Enter | Accept a function suggestion |

### Find

| Keys | Action |
|---|---|
| ⌘F / ⌥⌘F | Find / Find & replace |
| ⌘G / ↩ / ⇧↩ | Next / previous match (in the find bar) |

### Cells

| Keys | Action |
|---|---|
| ⌘ + click a link | Open the link (a plain click selects) |
| Click ▾ in a cell | Choose from the validation list |
| ⌥ + arrows | Change the row height / column width (⇧ sets the optimal size) |
| ⌃⌘+ / ⌃⌘− | Insert / delete cells… |

## Deliberate deviations from Apache OpenOffice

- ⌘D stays Fill Down (Excel and macOS usage); the AOO selection list is ⌃D.
- ⌘/ opens the shortcut reference; the AOO Select Array Formula is ⌘/ on the numeric keypad.
- ⌘⇧F9 is the hard recalculation; plain F9 recalculates the dirty cells (AutoCalculate is always on).
- ⌘K opens the command palette (AOO: ⌘K inserts a hyperlink, here ⌥⌘K).
- F4 (data source browser), F7 (spelling) and ⌥F11 (macros) are not bound: those features are out of scope (docs/parity-deviations.md).
