# ScrapMap

A portable Windows overlay that draws a live map for Scrap Mechanic: a compact
minimap pinned to the game window, expandable to a full map, with fog of war,
markers, points of interest and player positions.

Private project for two people. It does not need enterprise security, an
installer, or elaborate architecture — but it does touch a running game, so the
constraints below are real.

**Repository language is English**, and so is the interface. Docs, comments,
commit messages and identifiers are English.

Interface strings live in `public/map/locales/<code>.json`, keyed like
`SESSION_REVEAL_ALL`. English is the default and the fallback; a key missing
from a translation falls back to English rather than rendering blank. Adding a
language is a new file plus an entry in `LANGUAGES` in `public/map/i18n.js` --
no other code changes. **Do not put literal user-facing text in markup or
scripts**; add a key.

The tray menu is built before a WebView exists, so it reads the same
dictionaries at compile time through `include_str!` and takes the language from
`%LOCALAPPDATA%\ScrapMap\language.txt`, which the panel writes. That means the
tray follows the panel one restart later.

## Layout

| Path | What it is |
|---|---|
| `src-tauri/` | Rust host: window tracking, storage, atlas conversion, capture |
| `public/map/` | The shipping renderer — vanilla JS, no build step |
| `src/` | A TypeScript domain layer used only by tests; see `public/map/CLAUDE.md` |
| `game-patch/` | Lua installed into the game; canonical copies live here |
| `tools/` | `game-patch.mjs` (install/revert), repository hygiene check |
| `public/map/locales/` | Interface strings; English is the default and fallback |
| `docs/` | Architecture, game integration, sync design, roadmap |

## Commands

```bash
pnpm tauri build --no-bundle   # portable EXE -> src-tauri/target/release/scrapmap.exe
pnpm test                      # frontend tests
pnpm check:hygiene             # blocks local paths and credentials from the repo
cargo test --manifest-path ./src-tauri/Cargo.toml
node tools/game-patch.mjs status|snapshot|apply|revert
```

Use `pnpm tauri build`, not `cargo build` — a plain cargo build produces an EXE
that tries to reach a dev server instead of the bundled frontend.

## Things that will cost you a day if you do not know them

These were all learned the hard way. Each one produced a bug that looked like
something else entirely.

**The game must run with `-dev`.** Without it, edited Lua is ignored. Launch
through Steam; running `ScrapMechanic.exe` directly fails with
`SteamAPI_init failed`.

**Scrap Mechanic verifies Lua script checksums when joining a server.** The host
sends `m_serverGameInfo.m_vecFileChecksums` and the client compares its own
files. Patched scripts therefore only work when *every* player in the session
has byte-identical files, or the client is refused with `Invalid checksum`.
`tools/game-patch.mjs` exists for this reason, and normalises line endings when
installing because `core.autocrlf` on a fresh clone otherwise rewrites the files
and breaks the match.

**Lua globals survive a world reload.** Addon files guard with
`g_scrapMap...Installed`, so re-running the file is a no-op and a *changed*
addon does not take effect until the game is fully restarted. A world reload is
not enough. This has masked several fixes.

**`sm.json.fileExists` cannot see files written during the same session.** The
game indexes its virtual filesystem at startup. Anything ScrapMap writes while
the game runs is invisible to that call — so it cannot be used to detect a
request file or to check whether output already exists. Track state in game
storage instead, or just attempt the read and let it fail.

**`sm.json.save` writes Lua tables the way Lua sees them.** Every number is a
double (`512.0`, not `512`), and an empty table serialises as `null`, not `[]`.
Rust deserialisers must tolerate both; serde's `default` only covers a *missing*
field, not a null one.

**Some shipped game JSON is really JSONC.** `farming.harvestableset` carries
`//` comments. Strict parsers reject the whole file silently.

**Terrain streams around the player, not the camera.** Moving the camera alone
shows sky wherever the player is not. This is why the POI photography sweep only
worked near the player, and why it now teleports the player too, via
`SurvivalGame.sv_e_recreatePlayerInWorld` — the game's own travel path, which
loads the destination cell *before* recreating the character there.

**`sm.physics.raycast` returns a userdata, not a table.** Guarding its result
with `type( result ) == "table"` is always false. This silently disabled both
probes in the POI sweep for three full runs, which framed every photograph from
sea level and never pulled the camera back for a single tower — while reporting
nothing worse than a probe "miss". Prefer measuring from baked atlas data over
casting rays at all; see `docs/HANDOFF.md`.

**A handful of tiles always capture as one flat colour, and nothing yet
explains it.** Four of 116 reject as `detail 0.0-0.2` on every attempt, across
fresh sessions, three capture retries and a doubled settle. The obvious suspect
was the renderer: `RenderScene.cpp:183 Oh no! we are out of culling groups!!!`
appears up to 670,000 times in a long session. It is not the cause -- a session
with **zero** culling errors reproduced the same failures. Do not spend the
theory again without new evidence. Those tiles fall back to their generated
image, which is a perfectly good outcome.

**`sm.render.setCinematic( true )` draws letterbox bars.** It cost the top and
bottom tenth of all 116 POI photographs. `sm.gui.hideGui` is what hides the HUD.

**Applying a dictionary rewrites every `data-i18n` element.** Marking something
the code writes at runtime replaces the live value with the initial one, and the
dictionary loads asynchronously so it wins that race. The profile summary sat
permanently on its placeholder for exactly this reason. Runtime text goes
through `SMText.t` where it is written, never through markup.

**The panel has no console in a release build.** It reports to
`%LOCALAPPDATA%\ScrapMap\ui.log`: uncaught errors with a file and line, and the
profile state machine's decisions. Two wrong diagnoses were spent guessing at a
stuck profile before this existed; the log named the cause on the first run.

**`BitBlt` returns pure black on the game window** because it is
DirectX-presented. `PrintWindow` with `PW_RENDERFULLCONTENT` returns a real
frame. `cargo run --example capture_probe` re-answers this in one command, which
is worth doing after a game or driver update.

## Working style

Verify against the game's own data rather than assuming. Nearly every wrong turn
in this project came from a plausible assumption that one grep would have
disproved — that loose Lua was ignored, that `getColorAt` returned a colour,
that collision meshes were in metres. The game ships its databases as readable
JSON; read them.

Measure before and after a rendering change. Decode the actual PNG rather than
trusting that it looks right — and beware that a hand-rolled PNG reader must
handle per-row filters, or it will report convincing nonsense.

Keep the caches disposable. Everything under `%LOCALAPPDATA%\ScrapMap\atlas` can
be deleted and rebuilt; prefer fixing the generator over patching its output.

## Safety constraints

- No Cheat Engine, no writes to game process memory, no DirectX hook.
- Read-only with respect to gameplay: never touch inventory, health, crafting or
  server-controlled state. **One recorded exception:** the POI photography sweep
  turns god mode on for its duration and restores the previous value afterwards.
  It holds the player's controls for a quarter of an hour hundreds of metres up,
  and four runs ended with the player knocked out by the fall. Agreed with the
  user rather than assumed; nothing else may follow it without the same.
- Portable EXE only. No MSI/NSIS, no service, no auto-update, no listening port.
- Do not redistribute the game's own art or scripts. `game-patch/vanilla/` is
  gitignored for this reason — it holds verbatim copies of Axolot's files as a
  restore baseline, recreated locally with `snapshot`.
- Any game update must disable an incompatible bridge safely rather than crash.

## Reference

Game version of record: **Scrap Mechanic 1.0.4.874**, Windows x64. The install
is discovered automatically in the usual Steam library locations; override with
`--game=<path>` or `SCRAPMAP_GAME_ROOT`.

- `docs/ARCHITECTURE.md` — how the pieces fit, identity and storage rules
- `docs/GAME-INTEGRATION.md` — the Lua patch, atlas bake, POI photography
- `docs/SYNC.md` — shared fog and markers design (not started)
- `docs/ROADMAP.md` — milestones and what remains
- `docs/MULTIPLAYER-TEST.md` — instructions for the second player
