# Diagnostic telemetry feed

**Legacy.** This describes a JSON file input that predates the current
telemetry path. Live player positions now come from the game log, written by
`game-patch/Survival/Scripts/game/ScrapMapTelemetry.lua` and parsed by
`src-tauri/src/game_log_source.rs` — see `GAME-INTEGRATION.md`.

Nothing in this repository writes the file described below. The reader
(`diagnostic_source.rs`, ~1000 lines) is still compiled and still started, but
sits behind the log source as a fallback that never fires. It is documented here
so the decision to keep or remove it is an informed one; see the dead-code
section of `ARCHITECTURE.md`.

It remains useful for one thing: feeding the overlay synthetic telemetry without
running the game.

## File selection

In priority order:

1. `--telemetry-file PATH` or `--telemetry-file=PATH`
2. `SCRAPMAP_TELEMETRY_FILE`
3. `%LOCALAPPDATA%\ScrapMap\diagnostic\telemetry.json`

The resolved path stays in the native host. It never reaches the WebView and
never appears in a diagnostic event.

## Payload

```json
{
  "schemaVersion": 2,
  "worldId": "world-1",
  "sequence": 128,
  "localPlayerId": "76561198000000000",
  "staleAfterMs": 1500,
  "source": {
    "type": "example",
    "enumeration": "client-visible-players",
    "isHost": true,
    "compatibility": "supported"
  },
  "player": { "...": "the local player, repeated from players[]" },
  "players": [
    {
      "id": "76561198000000000",
      "name": "Player",
      "isLocal": true,
      "active": true,
      "hasCharacter": true,
      "sameWorld": true,
      "x": -2344.0, "y": -2585.6, "z": 21.5,
      "heading": 137.4
    }
  ]
}
```

A snapshot is published only when exactly one usable local player is present —
`active`, `hasCharacter`, `sameWorld`, and finite coordinates. A `worldId` that
disagrees with the active profile is rejected outright.

## Limits

| Bound | Value |
|---|---|
| Poll interval | 100 ms |
| Maximum file size | 1 MiB |
| Maximum players | 64 |
| Name / id / worldId length | 80 / 128 / 160 characters |
| Coordinate magnitude | ≤ 10,000,000 |
| Vector magnitude | ≤ 1,000,000 |
| `schemaVersion` | 1..=32 |
| Integer magnitude | ≤ 2^53 − 1 |

`heading` is normalised to 0–360. File size and metadata are checked before and
after reading, and identical content is not republished.

## Sequence and freshness

`sequence` must be a positive integer, strictly increasing within the current
profile activation. Missing, zero, repeated or smaller frames move nothing and
reveal no fog. Frames arriving during a profile switch are dropped.

Freshness is computed in the native host from monotonic receipt time and
`staleAfterMs`. The `timestamp` field is informational and cannot by itself hold
a stream `active` — a writer that stops writing goes stale regardless of what it
last claimed.

## Compatibility gate

`source.compatibility` of `unsupported` puts the host in an `unsupported` state:
no snapshot is published, the last good snapshot is not replaced, and players
and fog are unchanged. `supported` and `unknown` keep current behaviour.

This is a fail-closed boundary, not a substitute for a build fingerprint check.

## Writing the file

Write a temporary file and rename it onto `telemetry.json`. The reader tolerates
a plain partial overwrite — it will report `waiting`, `stale` or `invalid` while
keeping the last good snapshot — but an atomic rename avoids the situation
entirely.
