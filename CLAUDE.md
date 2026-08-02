# ScrapMap

A portable Windows overlay that draws a live map for Scrap Mechanic: a compact
minimap pinned to the game window, expandable to a full map, with fog of war,
markers, points of interest and player positions.

Private project for two people. It does not need enterprise security, an
installer, or elaborate architecture — but it does touch a running game, so the
constraints below are real.

**Repository language is English.** Docs, comments, commit messages and
identifiers are English. The overlay's own UI strings are Russian on purpose,
because that is what its users read; do not "fix" those.

## Layout

| Path | What it is |
|---|---|
| `src-tauri/` | Rust host: window tracking, storage, atlas conversion, capture |
| `public/map/` | The shipping renderer — vanilla JS, no build step |
| `src/` | A TypeScript domain layer used only by tests; see `public/map/CLAUDE.md` |
| `game-patch/` | Lua installed into the game; canonical copies live here |
| `tools/` | `game-patch.mjs` (install/revert), repository hygiene check |
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
worked near the player.

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
  server-controlled state.
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
