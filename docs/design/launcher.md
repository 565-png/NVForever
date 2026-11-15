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
- "Update to vX": when GitHub's newest release (the release list's first one, drafts left out;
  `releases/latest` skips pre-releases) is later than the launcher's version, a button that
  opens its page. Nothing is downloaded or installed (an updater is a later task).
- Continue (the newest save, with its picture, name, level and place: `--play --load`),
  Main menu (`--play --menu`), and More: New game (`--play --new-game`), the explorer
  (`--explore`) and "Close the launcher when the game starts" (`[Launcher] CloseOnPlay`).
  No `--data` is passed: the game reads its folder from the INI, so a launcher start is a
  player's start in every way (reports included, by the player's own choice).

## Rules it keeps

- Writes only NVForever's own files (the INI, `plugins.txt`), through
  `fnv_install::guard_write`; never the game folder or `Documents\My Games\FalloutNV`.
- Sends nothing: three GET requests (the site's news and config, GitHub's release list), only
  while "Fetch news" is on.
- Test aid: `NVF_LAUNCHER_SHOT=<file.png>` saves the window once the news has arrived and
  closes; `NVF_LAUNCHER_PAGE=home|settings|mods|game` opens that page first.

## Not yet

- What kind a row is, its default and a list's choices come from the INI and a short table, not
  from the game. Better: the game writes its settings catalogue (kinds, steps' words, choices,
  defaults) to the home at each start, and the launcher reads it.
- Screen readers: eframe's `accesskit` brings zbus on Linux (unmaintained, RustSec), so it is off.
- No folder picker on Linux (rfd's portal backend brings zbus too): the folder is typed.
