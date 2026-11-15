# Launcher (`nvforever-launcher`, #2708)

A small window beside `nvforever.exe` (`crates/nvf-launcher`, egui/eframe on OpenGL) that
shows the project's news, starts the game and changes the settings in `NVForever.ini` without
starting the game first [design: owner request 2026-11-15]. The game does not need it: a start
without the launcher is the same as before.

## Pages

- **Home**: news posts. First the site's `https://nv-forever.com/news.json`
  (`{"posts":[{"title","date","author","image","body" (Markdown),"url"}]}`, see
  `src/news.rs`), and while the site has none, this repository's GitHub releases (title, date,
  notes). The last list is kept in the home's cache folder (`launcher-news.json`) for offline
  starts. "Fetch news" (`[Launcher] News=0/1`) turns every request off.
- **Settings**: the `[Settings]` rows by the in-game page's tabs and headings, with its labels
  and notes, read from `docs/settings-reference.md` (built in). The reference does not say what
  kind a row is, so the launcher goes by the value in the INI: `1`/`0` a switch, `0.50` a
  slider (as a percentage, or the step's words for Difficulty), anything else text; Kill camera
  and V.A.T.S. style get their choices as a list (a test checks the engine still has them). A
  row the game has not written yet shows "game default". Rows the reference does not list go
  under Other. Save writes only the changed keys over the file as it is on disk then.
- **Mods**: NVForever's own `plugins.txt` (turn on/off, move). Opening the page runs
  `fnv_install::plugins::sync`, as the game's start does, so new plugins get their lines.
- **Game folder**: `[Game] Path` with the required-files check, the program to start
  (`[Launcher] Exe`, else `nvforever(.exe)` beside the launcher, else this checkout's
  `engine/target/release`), and buttons that open the saves, logs, screenshots and settings
  folders.
- **Website / GitHub**: open nv-forever.com and the repository in the browser.

## Top and bottom bars

- The site's next-release countdown (the `nextRelease` block of the site's
  `assets/js/config.js`, the same that drives the site's own bar), in the middle of the top
  bar; nothing when the site announces none or the moment has passed.
- "Update to vX": when GitHub's newest release (the highest version of the release list,
  drafts left out; `releases/latest` skips pre-releases) is later than the launcher's version,
  a button that opens the update dialog (see Updater).
- Continue (the newest save, with its picture, name, level and place: `--play --load`),
  Main menu (`--play --menu`), and More: New game (`--play --new-game`), the explorer
  (`--explore`) and "Close the launcher when the game starts" (`[Launcher] CloseOnPlay`).
  No `--data` is passed: the game reads its folder from the INI, so a launcher start is a
  player's start in every way (reports included, by the player's own choice).

## Fonts

The system's own interface font, read from its font folder at start and never shipped (Windows:
Segoe UI, Semibold for titles; Linux: Noto Sans or DejaVu Sans when installed), else egui's
built-in fonts. Their taller lines, a 16-point news body and wider paragraph spacing answer the
contributors' note that the news read "very squished together" (2026-11-15). egui has no
line-height setting for Markdown text, so the room comes from the font and its size.

## Rules it keeps

- Writes only NVForever's own files (the INI, `plugins.txt`), through
  `fnv_install::guard_write`; never the game folder or `Documents\My Games\FalloutNV`.
- Sends nothing: three GET requests (the site's news and config, GitHub's release list), only
  while "Fetch news" is on; an update adds two more (the release's `SHA256SUMS.txt` and its
  archive), only when the player presses Update now.
- Test aid: `NVF_LAUNCHER_SHOT=<file.png>` saves the window once the news has arrived and
  closes; `NVF_LAUNCHER_PAGE=home|settings|mods|game` opens that page first (`update`: the
  update dialog over Home, once a newer release is known); `NVF_LAUNCHER_RELEASES=<url>` reads
  the release list from that address instead of GitHub (a local server with a fixture release,
  to try the updater).

## Updater (#2713)

`src/update.rs`; the deciding steps are pure functions with tests, the acting ones are tested
on fixture folders and a fixture zip/tar.gz (no real release is downloaded by any test).

- **Check.** Only while "Fetch news" is on (`[Launcher] News`): the release list is fetched
  with the news, so turning news off turns update checks off too. The newest release is the
  highest version of the list, drafts left out (every release so far is a pre-release, so
  `releases/latest` answers 404); it counts when it is later than the launcher's own version
  (the workspace version, the same as the game's in a release archive).
- **Which file.** The archive for this build's target, by the exact name the release workflow
  packs (`.github/workflows/release.yml`): `NVForever-<tag>-x86_64-pc-windows-msvc.zip` on
  Windows, `NVForever-<tag>-x86_64-unknown-linux-gnu.tar.gz` on Linux. Another system, or a
  release without that file, gets the release page only.
- **Integrity.** Every release carries `SHA256SUMS.txt` (the publish job's `sha256sum *`). The
  launcher reads it first, hashes the archive as it arrives and compares; no list, no line for
  the archive, or a different hash: nothing is installed (the partial file is deleted). This
  catches a broken or cut download, not a release replaced on GitHub itself (the list comes
  from the same place); signing the list (minisign or similar) would be the next step.
- **Staging.** `<cache>/updates/<tag>/` in NVForever's cache folder (`EngineHome::cache`, the
  local data folder on Windows): `<archive>.part` while it downloads, the archive once checked
  (a second Update now with a checked archive there skips the download), `unpacked/`. The
  folder goes through `guard_write` (never the game folder or `My Games\FalloutNV`); the tag
  comes from the network, so it must be letters, digits, `.`, `_`, `-`. Unpacking refuses an
  entry that would land outside (`..`, absolute, a drive), links and devices, and an archive
  without the launcher and `nvforever` in it. Staging folders of versions no newer than the
  running one are removed at the launcher's start.
- **Where it installs.** The folder the launcher runs from, wherever the player unpacked the
  archive. Refused (the dialog says why and offers the release page) when the launcher is a
  build of a source checkout (an ancestor has `engine/Cargo.toml`: update with git), when
  `nvforever(.exe)` is not beside it, or when the folder is inside the game's (`guard_write`).
  The home (saves, `NVForever.ini`, `plugins.txt`, mods, logs) is a different folder and is
  never touched: it is not in the archive, and the swap only moves the archive's files and the
  ones the last update listed.
- **The swap**, all or nothing (`update::plan`, then `update::apply`):
  1. Plan: copy every file of the unpacked release; first set aside each of those already in
     the install, and each file of `update/installed.json` (what the last update installed)
     the new release no longer has. A file the player put in the folder is never in either
     list. A payload path outside the install or in `update/` refuses the whole update.
  2. Set aside: rename into `<install>/update/previous/v<running version>/`. A rename within
     the install's own drive, so it is quick and allowed even for the running launcher's exe
     (Windows lets a running program be renamed, not overwritten); the backup is beside the
     install, not in the cache, because the cache may be on another drive.
  3. Copy each new file to `<name>.nvf-new` and rename it into place, so no file is ever half
     written in its place.
  4. Write `update/installed.json` (version, files).
  Any failure in 2-4 undoes what was done (copied files removed, set-aside files renamed
  back), so the install is as before and the dialog says so. Afterwards only the one previous
  version is kept under `update/previous/`; the staging folder is deleted.
- **Not while it runs.** The game, the viewer and fnvtool of the install must not be running:
  the launcher waits while the game it started runs ("installs as soon as the game is
  closed"), and before the swap opens each program for writing, which Windows (sharing
  violation) and Linux (ETXTBSY) refuse while it runs; one that is running stops the install
  with its name.
- **Restart.** After the swap the launcher starts the new `nvforever-launcher` in the install
  with `--updated` and closes; the new one says "Updated to vX".
- **Roll back.** The Game folder page's Version card shows "Roll back to vX" when
  `update/previous/v<X>/` holds the launcher and the game. It is the same swap with the kept
  folder as the payload, so the current version is kept in its place (a second roll back goes
  forward again); the launcher restarts with `--rolled-back`.
- **The dialog.** Version, download size, the release notes (Markdown), "Your saves, settings
  and mods are not touched", Update now / Later / Release page. Then a progress bar (MB of MB
  while downloading, Cancel), "Checking the download", "Unpacking", "Installing". Offline, a
  failed check or a failed swap: the reason, "The NVForever folder is as it was", Try again /
  Release page. Closing the dialog lets a download carry on; the Update button opens it again.
  Downloads use their own HTTP agent: 15 s to connect and 30 s for any one read, no limit on
  the whole, so a slow line finishes and a dead one ends in an error.
- **Crates.** `zip` 9 (MIT; deflate only, no AES/bzip2/lzma/zstd), `tar` 0.4 (MIT/Apache-2.0,
  no xattr), `flate2` on zlib-rs and `sha2`, both already in the workspace; `cargo deny`
  passes.

## Not yet

- What kind a row is, its default and a list's choices come from the INI and a short table, not
  from the game. Better: the game writes its settings catalogue (kinds, steps' words, choices,
  defaults) to the home at each start, and the launcher reads it.
- Screen readers: eframe's `accesskit` brings zbus on Linux (unmaintained, RustSec), so it is off.
- No folder picker on Linux (rfd's portal backend brings zbus too): the folder is typed.
- Updater: the checksum list is not signed; no automatic install without the player's click;
  no update of a launcher installed outside a release folder (a package manager's copy).
