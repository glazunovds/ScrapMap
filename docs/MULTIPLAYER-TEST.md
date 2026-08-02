# Playing together — instructions for the second player

Scrap Mechanic checks Lua script checksums when you join a server. The host
sends its list, your client compares its own files, and any difference is
refused with `Invalid checksum`. So **both players must run byte-identical
files** — if one of you is unpatched, or on a different revision, neither of you
gets in.

The patch is only needed for live player positions. The map itself works without
it.

## One-time setup

1. Restore clean game files: Steam → Scrap Mechanic → Properties → Installed
   Files → **Verify integrity of game files**. Do this before anything else, so
   the copies ScrapMap keeps to restore from are genuinely untouched.

2. Both of you download **the same version** of `scrapmap.exe`. Nothing to
   install, and no Node or checkout required — the Lua is inside the binary,
   which is what makes "the same version" mean "the same bytes".

3. Add `-dev` to Scrap Mechanic's launch options in Steam. Without it the game
   ignores the patched scripts entirely.

## Before playing together

Both of you: **Tray → Install game patch**.

ScrapMap records the untouched files the first time, so there is no separate
snapshot step. It writes the addons with normalised line endings, because a
checkout with `core.autocrlf` would otherwise change a byte and one byte is a
refused connection.

The one rule: **the same ScrapMap version on both machines.** Different versions
may carry different Lua, and the game compares checksums when you connect.

## To play with someone unpatched

**Tray → Restore game files.** The game returns to stock and you can join
anyone. The map, fog and markers keep working — the tile atlas is cached
locally and does not depend on the patch.

## If the connection is refused

`Invalid checksum` means the two installs disagree. In order of likelihood:

- different ScrapMap versions — compare them and use one;
- one of you has not applied the patch, or has applied it and not reloaded;
- your game versions differ, in which case verify files and start again.

## What the test is for

- Can the client join the host with the same patch applied?
- Are both players visible on the minimap with their names?
- Does the guest identify the world and profile correctly?
- Do players disappear from the map after they leave, rather than lingering?
- Does the arrow point where the player is actually facing?
