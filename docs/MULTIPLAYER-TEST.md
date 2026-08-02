# Playing together — instructions for the second player

Scrap Mechanic checks Lua script checksums when you join a server. The host
sends its list, your client compares its own files, and any difference is
refused with `Invalid checksum`. So **both players must run byte-identical
files** — if one of you is unpatched, or on a different revision, neither of you
gets in.

The patch is only needed for live player positions. The map itself works without
it.

## One-time setup

1. Install [Node.js](https://nodejs.org) — only to run the patch script.

2. Clone the repository:

   ```bash
   git clone https://github.com/glazunovds/ScrapMap.git
   ```

3. Restore clean game files: Steam → Scrap Mechanic → Properties → Installed
   Files → **Verify integrity of game files**.

4. Record the baseline the patch applies to and reverts to:

   ```bash
   node tools/game-patch.mjs snapshot
   ```

   The script finds the game in the usual Steam library locations. If it does
   not, add `--game="<path to the Scrap Mechanic folder>"`.

## Before playing together

Both of you, from the **same repository revision** — `git pull`, then check
`git log -1` shows the same commit:

```bash
node tools/game-patch.mjs apply
```

The script prints a hash per file. **Compare them.** All five must match. If any
differ you will not be able to connect.

If hashes disagree it is almost always line endings: git on Windows converts LF
to CRLF on checkout by default, which changes the bytes. `git pull` and run
`apply` again — the script normalises line endings on install, and
`.gitattributes` pins them in the repository.

If the three addon files match but `SurvivalGame.lua` or `terrain_overworld.lua`
differ, that is a different problem: those are rebuilt from each machine's own
game files, so a mismatch means your game versions differ.

Launch through Steam with `-dev` in the launch options — without it the game
ignores the patched scripts entirely.

## To play with someone unpatched

```bash
node tools/game-patch.mjs revert
```

Game files return to stock and you can join anyone. The map, fog and markers
keep working; the tile atlas is cached locally and does not depend on the patch.

## Checking state

```bash
node tools/game-patch.mjs status
```

## What the test is for

- Can the client join the host with the same patch applied?
- Are both players visible on the minimap with their names?
- Does the guest identify the world and profile correctly?
- Do players disappear from the map after they leave, rather than lingering?
- Does the arrow point where the player is actually facing?
