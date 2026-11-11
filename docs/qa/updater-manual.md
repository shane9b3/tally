# Manual QA: auto-update (owner)

What the automated tests cannot cover: a real update between two tagged
releases, signature checks against the real key, the macOS bundle swap and
relaunch, and the network. Run on macOS (Apple Silicon) at least; Windows
and Linux rows where a machine is available. Setup: docs/release.md
section 6 (two published releases `vA` < `vB`, `vA` installed in
`/Applications`).

Record: date, machine / OS version, `vA` → `vB`, result per row, notes.

## A. Automatic prompt and the happy path

| # | Steps | Expected |
|---|---|---|
| A1 | Launch `vA` with one empty document. Wait 10 s. | One prompt, in the front window: "Tally vB is available — you have vA", the release date, the release notes of `vB` as plain text (Markdown/HTML characters shown as typed), buttons Skip This Version / Later / Install and Restart. Install and Restart has the focus. |
| A2 | Open a second window (File ▸ New with "Open documents in: Separate windows") and two tabs, relaunch `vA`. | Still exactly one prompt (front window), not one per document. |
| A3 | Keyboard only: Tab cycles inside the prompt; the notes area scrolls with the arrow keys; Escape closes it (= Later). VoiceOver reads the title and the notes. | As described. |
| A4 | Later, then **Tally ▸ Check for Updates…** (the macOS app menu; Help ▸ Check for Updates… on Windows and Linux). | The prompt again. |
| A5 | Install and Restart. | "Downloading Tally vB…" with a progress bar and percentage; no buttons; Escape does nothing. Then the window closes and Tally relaunches as `vB` (Options ▸ Online Update ▸ "Installed version: Tally vB"). No Gatekeeper prompt on the relaunch. |
| A6 | After A5: `codesign -dv /Applications/Tally.app`; `xattr -l /Applications/Tally.app`. | Ad-hoc signature (`Signature=adhoc`); no `com.apple.quarantine`. |

## B. Unsaved documents

| # | Steps | Expected |
|---|---|---|
| B1 | `vA` with three documents, two of them edited (one in another window). Check for Updates… ▸ Install and Restart. | After the download the update prompt closes and the edited documents are asked in turn (front window first): "Do you want to save the changes…?" Save / Don't Save / Cancel, as for Quit. |
| B2 | Answer Save for the first (pick a file), Don't Save for the second. | Tally installs and relaunches as `vB`. The saved file has the changes; the discarded document is not offered for recovery on the next launch. |
| B3 | Repeat B1, answer Cancel at the second document. | Nothing closes. "Restart cancelled" explains that `vB` is downloaded but not installed and every document is still open. The first document's answer no longer counts (closing it asks again). |
| B4 | In the "Restart cancelled" dialog choose Install and Restart. | No second download; the questions start again. |
| B5 | After B3 choose Later, then Quit Tally (⌘Q, answer the prompts). Relaunch. | Still `vA`; the update is offered again (it was only kept in memory). |
| B6 | During B1 press ⌘Q while a save question is open. | The restart turns into a normal Quit (Tally quits, the update is not installed). |

## C. Skip, Later, the switch

| # | Steps | Expected |
|---|---|---|
| C1 | At the automatic prompt choose Skip This Version. Relaunch `vA`. | No prompt (wait 30 s). |
| C2 | Check for Updates… | The prompt for `vB` (a manual check ignores the skip). |
| C3 | Publish `vC` > `vB` (or wait for the next real release). Relaunch `vA`. | The prompt for `vC`. |
| C4 | Options ▸ Online Update: turn "Check for updates automatically" off, OK. Relaunch. | No automatic prompt; Check for Updates… still works. Turn it back on. |
| C5 | Later at the prompt, leave Tally running over 24 h (or note the next day). | The prompt comes back once a day while the switch is on. |

## D. Failures

| # | Steps | Expected |
|---|---|---|
| D1 | Turn Wi-Fi off. Launch `vA`, wait 30 s. | No prompt, no error (automatic checks fail silently). |
| D2 | Still offline: Check for Updates… | "Tally could not check for updates" with the reason (after at most 30 s); OK closes it. |
| D3 | Start Install and Restart, then turn Wi-Fi off during the download (use a slow link if the download is too fast). | "The update could not be downloaded" with the reason; nothing installed; documents untouched. |
| D4 | On the latest `vB`: Check for Updates… | "Tally is up to date — You have the newest version, vB." |
| D5 | Copy `Tally.app` to a folder you cannot write without an administrator (e.g. owned by root), run it, update. | macOS asks for an administrator password; Cancel there shows "The update could not be installed"; documents stay open. |

## E. Other platforms (when available)

| # | Steps | Expected |
|---|---|---|
| E1 | Windows: install `vA` with the `-setup.exe` (SmartScreen: More info ▸ Run anyway). Update to `vB`. | Prompt as A1. Install and Restart: the questions, then a small installer progress window (passive), and Tally relaunches as `vB`. No SmartScreen prompt for the update. |
| E2 | Windows: an edited document answered Don't Save before the update. | Not offered for recovery after the relaunch. |
| E3 | Linux AppImage: `chmod +x`, run `vA`, update. | The AppImage file is replaced in place and relaunches as `vB`. |
| E4 | Linux `.deb` install of `vA`: Check for Updates… | "Tally could not check for updates: This copy of Tally was not installed as an AppImage…"; Options ▸ Online Update shows the same reason. |
