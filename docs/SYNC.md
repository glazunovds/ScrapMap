# Shared fog and markers

**Status: not started.** This is the intended design, not a description of
running code. `src/sync/` contains an interface declaration and nothing else,
and there is no Worker in the repository.

## Shape

Cloudflare renders nothing and stores no game assets. One Worker routes requests
into a SQLite-backed Durable Object selected by an opaque room id. That object
serialises changes to shared fog and markers and survives eviction or redeploy
without a VPS process to babysit.

No D1, KV or R2 for the prototype, and no WebSockets. Hibernating WebSocket
presence is explicitly deferred and must not block the first cooperative
checkpoint — polling is sufficient for two players.

## API v1

```
GET    /healthz
GET    /v1/rooms/{room}/snapshot
GET    /v1/rooms/{room}/changes?after=<revision>
PUT    /v1/rooms/{room}/fog
PUT    /v1/rooms/{room}/markers/{marker-id}
DELETE /v1/rooms/{room}/markers/{marker-id}
```

Every mutation carries an `Idempotency-Key`, and every response returns the new
server revision. A client connects with a snapshot and then follows a revision
cursor.

Polling: fog deltas batched every few seconds, marker mutations sent
immediately. Five seconds is enough for both.

## Fog

A grow-only set, unioned monotonically in the Durable Object. Only cells around
the local player are sent.

**Player coordinates and breadcrumbs are never sent to Cloudflare.** Fog says
where someone has been, which is the point; live position is nobody else's
business and is not needed for the feature.

## Markers

Stable UUID per marker. Update and delete carry the expected version; a conflict
returns `409` with the current record so the client can reconcile. Delete
creates a tombstone. Server revision defines ordering — no client clocks, which
avoids every clock-skew argument.

Both players may add and delete shared markers.

## Access

A random room id and one bearer token per device, sent in `Authorization` and
never in a query string, never in a diagnostic log. An unknown token gets an
identical `401` whether or not the room exists, so the endpoint cannot be used
to probe for rooms.

The world fingerprint and profile key stay local; a room is bound to them only
after an explicit join.

## Boundaries

- Bounded JSON body and bounded fog batch.
- Coordinates validated against world bounds.
- `Cache-Control: no-store`.
- No admin panel, no OAuth, no role model.
- Must survive local Miniflare eviction and a Cloudflare redeploy.

## Client side

A local SQLite outbox, so offline work replays when the room is reachable again.
Replay must be idempotent — that is what the `Idempotency-Key` is for.
