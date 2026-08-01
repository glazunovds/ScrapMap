import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const bridgeSource = readFileSync(
  new URL("../public/map/overlay-bridge.js", import.meta.url),
  "utf8",
);

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

async function waitFor(predicate, message = "condition was not reached") {
  for (let attempt = 0; attempt < 200; attempt += 1) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  throw new Error(message);
}

class TinyEventTarget {
  #listeners = new Map();

  addEventListener(type, listener) {
    const listeners = this.#listeners.get(type) || [];
    listeners.push(listener);
    this.#listeners.set(type, listeners);
  }

  dispatchEvent(event) {
    for (const listener of this.#listeners.get(event.type) || []) {
      listener.call(this, event);
    }
    return true;
  }
}

class TinyCustomEvent {
  constructor(type, options = {}) {
    this.type = type;
    this.detail = options.detail;
  }
}

function layoutContext(sourceWorldId) {
  return {
    schemaVersion: 1,
    sourceWorldId,
    gameMode: "unknown",
    cellSize: 64,
    bounds: { minX: 0, maxX: 2, minY: 0, maxY: 2 },
    cells: [
      {
        x: 0,
        y: 0,
        tileUuid: "00000000-0000-0000-0000-000000000001",
        rotation: 0,
        xOffset: 0,
        yOffset: 0,
        flags: 0,
      },
      {
        x: 1,
        y: 0,
        tileUuid: "00000000-0000-0000-0000-000000000002",
        rotation: 1,
        xOffset: 0,
        yOffset: 0,
        flags: 0,
      },
    ],
  };
}

function activatedSnapshot(request) {
  const sourceWorldId = request.layout.worldId;
  const suffix = sourceWorldId.replaceAll(/[^a-z0-9]+/gi, "-");
  const worldFingerprint = `smwf1-${suffix}`;
  return {
    schemaVersion: 1,
    profile: {
      schemaVersion: 1,
      profileKey: `smp1-${suffix}`,
      worldFingerprint,
      scopeKind: "local",
      scopeId: "default",
      identityQuality: "stable",
      gameMode: "unknown",
      serverKind: "local",
      serverStableId: null,
      displayName: null,
      needsManualDisambiguation: false,
    },
    sessionId: request.sessionId,
    settings: {
      schemaVersion: 1,
      fogEnabled: true,
      poiEnabled: [
        "schematic",
        "quest",
        "camp",
        "warehouse",
        "service",
        "dungeon",
        "landmark",
      ],
    },
    visited: { schemaVersion: 1, worldId: worldFingerprint, visited: [] },
    markers: { schemaVersion: 1, worldId: worldFingerprint, markers: [] },
    activeRoute: null,
    recentTrail: null,
  };
}

function remoteSnapshot(request) {
  const manual = Boolean(request.fallbackProfileId);
  const worldFingerprint = `smwf1_${"a".repeat(64)}`;
  const profileSuffix = manual
    ? request.fallbackProfileId.replaceAll(/[^a-z0-9]+/gi, "-")
    : "unresolved";
  return {
    ...activatedSnapshot(request),
    profile: {
      schemaVersion: 1,
      profileKey: `smp1-${profileSuffix}`,
      worldFingerprint,
      scopeKind: "fallback",
      scopeId: manual ? request.fallbackProfileId : "unknown:default",
      identityQuality: manual ? "manual" : "fingerprint-only",
      gameMode: "unknown",
      serverKind: "unknown",
      serverStableId: null,
      displayName: manual ? request.fallbackProfileName || null : null,
      needsManualDisambiguation: !manual,
    },
    visited: { schemaVersion: 1, worldId: worldFingerprint, visited: [] },
    markers: { schemaVersion: 1, worldId: worldFingerprint, markers: [] },
  };
}

function routeFixture(worldFingerprint, id = "route-one") {
  return {
    kind: "scrapmap.route",
    schemaVersion: 1,
    id,
    world: { worldFingerprint },
    generatedAt: "2026-07-30T12:00:00Z",
    strategy: "direct",
    status: "ready",
    start: {
      kind: "point",
      referenceId: null,
      label: null,
      position: { x: 0, y: 0, z: 0 },
    },
    destination: {
      kind: "point",
      referenceId: null,
      label: "Цель",
      position: { x: 64, y: 0, z: 0 },
    },
    path: [
      { x: 0, y: 0, z: 0 },
      { x: 64, y: 0, z: 0 },
    ],
    pathCells: [
      { x: 0, y: 0 },
      { x: 1, y: 0 },
    ],
    directDistanceWorldUnits: 64,
    routeDistanceWorldUnits: 64,
  };
}

function trailBatchFixture(trailId = "trail-one") {
  return {
    trailId,
    startedAtMs: 1_700_000_000_000,
    endedAtMs: null,
    points: [
      {
        sequence: 0,
        capturedAtMs: 1_700_000_000_000,
        world: { x: 0, y: 0, z: 0 },
        breakBefore: true,
      },
    ],
  };
}

function createHarness(options = {}) {
  const eventTarget = new TinyEventTarget();
  const tauriListeners = new Map();
  const calls = [];
  const hydrations = [];
  const handlers = {
    activate: options.activate || ((request) => activatedSnapshot(request)),
    listManualProfiles:
      options.listManualProfiles ||
      ((request) => ({
        schemaVersion: 1,
        worldFingerprint: request.worldFingerprint,
        candidates: [],
      })),
    mergeFog: options.mergeFog || (() => ({ inserted: 0, total: 0 })),
    replaceMarkers: options.replaceMarkers || (() => ({ stored: 0 })),
    saveSettings:
      options.saveSettings ||
      ((request) => request.settings),
    setActiveRoute:
      options.setActiveRoute ||
      ((request) => request.route),
    writeTrailBatch:
      options.writeTrailBatch ||
      ((request) => ({
        trailId: request.trailId,
        appended: request.points.length,
        total: request.points.length,
        endedAtMs: request.endedAtMs,
      })),
  };
  const acceptedTelemetrySequences = new Map();
  const localStorageEntries = new Map(
    Object.entries(options.localStorage || {}),
  );
  let uuidSequence = 0;
  let currentContext = options.context || layoutContext("demo-a");
  const ui = {
    worldId: currentContext.sourceWorldId,
    activeProfileKey: null,
    visited: [],
    localMarkers: [],
    sharedMarkers: [],
    poiCategories: ["schematic", "quest"],
    fogEnabled: true,
    persistenceStatus: "pending",
    telemetry: null,
    telemetryUpdates: [],
    activeRoute: null,
    recentTrail: null,
  };

  const minimap = {
    getProfileContext() {
      return currentContext;
    },
    getPersistenceStatus() {
      return {
        sourceWorldId: ui.worldId,
        status: ui.persistenceStatus,
      };
    },
    beginProfileHydration(sourceWorldId) {
      assert.equal(sourceWorldId, ui.worldId);
      ui.persistenceStatus = "pending";
    },
    hydrateProfileState(payload) {
      hydrations.push(structuredClone(payload));
      ui.activeProfileKey = payload.profileKey;
      ui.visited = [...(payload.visited || [])];
      ui.localMarkers = [...(payload.localMarkers || [])];
      ui.sharedMarkers = [...(payload.sharedMarkers || [])];
      ui.poiCategories = [
        ...(payload.settings?.poiCategories || ui.poiCategories),
      ];
      ui.fogEnabled = payload.settings?.fogEnabled !== false;
      ui.activeRoute = structuredClone(payload.activeRoute ?? null);
      ui.recentTrail = structuredClone(payload.recentTrail ?? null);
      ui.persistenceStatus = "ready";
    },
    getSnapshot() {
      return {
        worldId: ui.worldId,
        visited: [...ui.visited],
        markers: [
          ...ui.localMarkers.map((marker) => ({ ...marker, local: true })),
          ...ui.sharedMarkers.map((marker) => ({ ...marker, local: false })),
        ],
        localMarkers: ui.localMarkers.map((marker) => ({ ...marker })),
        sharedMarkers: ui.sharedMarkers.map((marker) => ({ ...marker })),
        poiCategories: [...ui.poiCategories],
        fogEnabled: ui.fogEnabled,
        activeRoute: structuredClone(ui.activeRoute),
        recentTrail: structuredClone(ui.recentTrail),
      };
    },
    updateTelemetry(payload) {
      ui.telemetry = structuredClone(payload);
      ui.telemetryUpdates.push(structuredClone(payload));
    },
    setTelemetryStatus() {},
    setExpanded() {},
  };

  async function invoke(name, args = {}) {
    calls.push({ name, args: structuredClone(args) });
    switch (name) {
      case "profile_activate":
        return handlers.activate(args.request);
      case "profile_list_manual_profiles":
        return handlers.listManualProfiles(args.request);
      case "profile_merge_fog":
        return handlers.mergeFog(args.request);
      case "profile_replace_local_markers":
        return handlers.replaceMarkers(args.request);
      case "profile_save_settings":
        return handlers.saveSettings(args.request);
      case "profile_set_active_route":
        return handlers.setActiveRoute(args.request);
      case "profile_write_trail_batch":
        return handlers.writeTrailBatch(args.request);
      case "profile_accept_telemetry_sequence": {
        if (options.acceptTelemetry) {
          return options.acceptTelemetry(args.request);
        }
        const sessionId = args.request.context.sessionId;
        const previous = acceptedTelemetrySequences.get(sessionId) || 0;
        if (args.request.sequence <= previous) return false;
        acceptedTelemetrySequences.set(sessionId, args.request.sequence);
        return true;
      }
      case "diagnostic_snapshot":
        return options.diagnosticActive
          ? {
              sequence: options.diagnosticSequence || 1,
              worldId: currentContext.sourceWorldId,
              source: { isHost: options.diagnosticIsHost === true },
            }
          : null;
      case "diagnostic_status":
        return { state: options.diagnosticActive ? "active" : "waiting" };
      case "game_layout_snapshot":
        return null;
      case "server_identity_probe":
        return typeof options.serverIdentityProbe === "function"
          ? options.serverIdentityProbe()
          : (
            options.serverIdentityProbe || {
            identity: { kind: "unknown", stableId: null },
            observationId: null,
            issue: "no-connection-identity",
          }
          );
      case "overlay_status":
        return { expanded: false, game: { state: "missing" } };
      case "set_overlay_mode":
        return null;
      default:
        throw new Error(`Unexpected invoke command ${name}`);
    }
  }

  const body = {
    dataset: {},
    classList: { toggle() {} },
  };
  const window = {
    __TAURI__: {
      core: { invoke },
      event: {
        async listen(name, listener) {
          const listeners = tauriListeners.get(name) || [];
          listeners.push(listener);
          tauriListeners.set(name, listeners);
          return () => {
            const index = listeners.indexOf(listener);
            if (index >= 0) listeners.splice(index, 1);
          };
        },
      },
    },
    SMMinimap: minimap,
    crypto: {
      randomUUID() {
        uuidSequence += 1;
        return `00000000-0000-4000-8000-${String(uuidSequence).padStart(12, "0")}`;
      },
    },
    setTimeout(callback) {
      return setTimeout(callback, 0);
    },
    setInterval() {
      return 1;
    },
    clearTimeout,
    localStorage: {
      getItem(key) {
        return localStorageEntries.has(key)
          ? localStorageEntries.get(key)
          : null;
      },
      setItem(key, value) {
        localStorageEntries.set(key, String(value));
      },
    },
    addEventListener: eventTarget.addEventListener.bind(eventTarget),
    dispatchEvent: eventTarget.dispatchEvent.bind(eventTarget),
  };
  window.window = window;

  vm.runInNewContext(bridgeSource, {
    window,
    document: { body },
    CustomEvent: TinyCustomEvent,
    console: options.console || console,
    structuredClone,
    setTimeout,
    clearTimeout,
  });

  return {
    body,
    calls,
    handlers,
    hydrations,
    localStorageEntries,
    ui,
    profiles: window.ScrapMapProfiles,
    setContext(context) {
      currentContext = context;
      ui.worldId = context.sourceWorldId;
      ui.activeProfileKey = null;
      window.dispatchEvent(
        new TinyCustomEvent("sm-minimap:profile-context-changed", {
          detail: context,
        }),
      );
    },
    setLocalMarkers(markers, profileKey = ui.activeProfileKey) {
      ui.localMarkers = structuredClone(markers);
      window.dispatchEvent(
        new TinyCustomEvent("sm-minimap:local-markers-replaced", {
          detail: {
            schemaVersion: 1,
            sourceWorldId: ui.worldId,
            profileKey,
            markers,
          },
        }),
      );
    },
    setSettings(settings, profileKey = ui.activeProfileKey) {
      ui.poiCategories = [...settings.poiCategories];
      ui.fogEnabled = settings.fogEnabled;
      window.dispatchEvent(
        new TinyCustomEvent("sm-minimap:settings-changed", {
          detail: {
            schemaVersion: 1,
            sourceWorldId: ui.worldId,
            profileKey,
            settings,
          },
        }),
      );
    },
    setActiveRoute(route, profileKey = ui.activeProfileKey) {
      ui.activeRoute = structuredClone(route);
      window.dispatchEvent(
        new TinyCustomEvent("sm-minimap:active-route-changed", {
          detail: {
            schemaVersion: 1,
            sourceWorldId: ui.worldId,
            profileKey,
            route,
          },
        }),
      );
    },
    writeTrailBatch(batch, profileKey = ui.activeProfileKey) {
      window.dispatchEvent(
        new TinyCustomEvent("sm-minimap:trail-batch", {
          detail: {
            schemaVersion: 1,
            sourceWorldId: ui.worldId,
            profileKey,
            batch,
          },
        }),
      );
    },
    reveal(cells) {
      window.dispatchEvent(
        new TinyCustomEvent("sm-minimap:fog-delta", {
          detail: {
            schemaVersion: 1,
            sourceWorldId: ui.worldId,
            cells,
          },
        }),
      );
    },
    emitTauri(name, payload) {
      for (const listener of tauriListeners.get(name) || []) {
        listener({ payload });
      }
    },
  };
}

test("A to B to A activation uses a fresh native write session each time", async () => {
  const harness = createHarness();
  await waitFor(() => harness.body.dataset.profileState === "ready");

  harness.setContext(layoutContext("demo-b"));
  await waitFor(
    () =>
      harness.calls.filter((call) => call.name === "profile_activate").length ===
        2 && harness.body.dataset.profileState === "ready",
  );
  harness.setContext(layoutContext("demo-a"));
  await waitFor(
    () =>
      harness.calls.filter((call) => call.name === "profile_activate").length ===
        3 && harness.body.dataset.profileState === "ready",
  );

  const sessions = harness.calls
    .filter((call) => call.name === "profile_activate")
    .map((call) => call.args.request.sessionId);
  assert.equal(new Set(sessions).size, 3);
});

test("marker and settings edits made during activation survive hydration", async () => {
  const activation = deferred();
  const harness = createHarness({ activate: () => activation.promise });
  await waitFor(() =>
    harness.calls.some((call) => call.name === "profile_activate"),
  );

  const marker = {
    id: "local-new",
    cellX: 1,
    cellY: 0,
    kind: "x",
    label: "Новая метка",
    createdAt: "2026-07-30T12:00:00Z",
  };
  harness.setLocalMarkers([marker]);
  harness.setSettings({ poiCategories: ["warehouse"], fogEnabled: false });
  const route = routeFixture("smwf1-demo-a", "route-during-activation");
  harness.setActiveRoute(route);

  const activationRequest = harness.calls.find(
    (call) => call.name === "profile_activate",
  ).args.request;
  activation.resolve(activatedSnapshot(activationRequest));
  await waitFor(() => harness.body.dataset.profileState === "ready");

  assert.deepEqual(harness.hydrations.at(-1).localMarkers, [marker]);
  assert.deepEqual(harness.hydrations.at(-1).settings.poiCategories, [
    "warehouse",
  ]);
  assert.equal(harness.hydrations.at(-1).settings.fogEnabled, false);
  assert.deepEqual(harness.hydrations.at(-1).activeRoute, route);

  const markerWrite = harness.calls.find(
    (call) => call.name === "profile_replace_local_markers",
  );
  const settingsWrite = harness.calls.find(
    (call) => call.name === "profile_save_settings",
  );
  const routeWrite = harness.calls.find(
    (call) => call.name === "profile_set_active_route",
  );
  assert.deepEqual(markerWrite.args.request.markers, [marker]);
  assert.deepEqual(settingsWrite.args.request.settings.poiEnabled, [
    "warehouse",
  ]);
  assert.equal(settingsWrite.args.request.settings.fogEnabled, false);
  assert.deepEqual(routeWrite.args.request.route, route);
});

test("active route and bounded trail batches hydrate and flush through the profile session", async () => {
  const harness = createHarness({
    activate(request) {
      const snapshot = activatedSnapshot(request);
      snapshot.activeRoute = routeFixture(
        snapshot.profile.worldFingerprint,
        "route-restored",
      );
      snapshot.recentTrail = {
        schemaVersion: 1,
        trailId: "trail-restored",
        sessionId: request.sessionId,
        startedAtMs: 1_700_000_000_000,
        endedAtMs: null,
        pointCount: 1,
        points: trailBatchFixture("trail-restored").points,
        truncated: false,
      };
      return snapshot;
    },
  });
  await waitFor(() => harness.body.dataset.profileState === "ready");

  assert.equal(harness.hydrations.at(-1).activeRoute.id, "route-restored");
  assert.equal(
    harness.hydrations.at(-1).recentTrail.trailId,
    "trail-restored",
  );

  const profile = harness.profiles.getState().profile;
  harness.setActiveRoute(routeFixture(profile.worldFingerprint, "route-new"));
  harness.writeTrailBatch(trailBatchFixture("trail-new"));
  await waitFor(
    () =>
      harness.calls.some((call) => call.name === "profile_set_active_route") &&
      harness.calls.some((call) => call.name === "profile_write_trail_batch"),
  );

  const routeWrite = harness.calls.find(
    (call) => call.name === "profile_set_active_route",
  ).args.request;
  const trailWrite = harness.calls.find(
    (call) => call.name === "profile_write_trail_batch",
  ).args.request;
  assert.equal(routeWrite.route.id, "route-new");
  assert.equal(trailWrite.trailId, "trail-new");
  assert.equal(
    routeWrite.context.sessionId,
    trailWrite.context.sessionId,
  );
});

test("same-layout profile switch cannot write marker or settings state into the old profile", async () => {
  const secondActivation = deferred();
  let activationCount = 0;
  const harness = createHarness({
    activate(request) {
      activationCount += 1;
      return activationCount === 1
        ? activatedSnapshot(request)
        : secondActivation.promise;
    },
  });
  await waitFor(() => harness.body.dataset.profileState === "ready");

  const changedContext = layoutContext("demo-a");
  changedContext.cells[1].rotation = 3;
  const writesBeforeSwitch = harness.calls.filter((call) =>
    [
      "profile_replace_local_markers",
      "profile_save_settings",
      "profile_set_active_route",
      "profile_write_trail_batch",
    ].includes(call.name),
  ).length;
  harness.setContext(changedContext);
  await waitFor(
    () =>
      harness.calls.filter((call) => call.name === "profile_activate").length ===
      2,
  );

  harness.setLocalMarkers(
    [
      {
        id: "stale-marker",
        cellX: 1,
        cellY: 0,
        kind: "x",
        label: "Не переносить",
        createdAt: "2026-07-30T12:00:00Z",
      },
    ],
    null,
  );
  harness.setSettings(
    { poiCategories: ["warehouse"], fogEnabled: false },
    null,
  );
  harness.setActiveRoute(routeFixture("smwf1-demo-a"), null);
  harness.writeTrailBatch(trailBatchFixture("stale-trail"), null);
  await new Promise((resolve) => setTimeout(resolve, 0));

  assert.equal(
    harness.calls.filter((call) =>
      [
        "profile_replace_local_markers",
        "profile_save_settings",
        "profile_set_active_route",
        "profile_write_trail_batch",
      ].includes(call.name),
    ).length,
    writesBeforeSwitch,
  );

  const activationRequest = harness.calls.filter(
    (call) => call.name === "profile_activate",
  )[1].args.request;
  secondActivation.resolve(activatedSnapshot(activationRequest));
  await waitFor(() => harness.body.dataset.profileState === "ready");
  assert.deepEqual(harness.hydrations.at(-1).localMarkers, []);
  assert.equal(harness.hydrations.at(-1).settings.fogEnabled, true);
});

test("telemetry is blocked during activation and sequence resets with the new session", async () => {
  const secondActivation = deferred();
  let activationCount = 0;
  const harness = createHarness({
    activate(request) {
      activationCount += 1;
      return activationCount === 1
        ? activatedSnapshot(request)
        : secondActivation.promise;
    },
  });
  await waitFor(() => harness.body.dataset.profileState === "ready");

  harness.emitTauri("scrapmap:telemetry", {
    sequence: 8,
    player: { x: 8, y: 8, z: 0 },
  });
  harness.emitTauri("scrapmap:telemetry", {
    sequence: 9,
    worldId: "demo-a",
    player: { worldId: "demo-b", x: 9, y: 9, z: 0 },
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(harness.ui.telemetryUpdates.length, 0);

  harness.emitTauri("scrapmap:telemetry", {
    sequence: 10,
    worldId: "demo-a",
    player: { worldId: "demo-a", x: 10, y: 20, z: 0 },
  });
  await waitFor(() => harness.ui.telemetryUpdates.length === 1);

  harness.emitTauri("scrapmap:telemetry", {
    sequence: 9,
    worldId: "demo-a",
    player: { worldId: "demo-a", x: 99, y: 99, z: 0 },
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(harness.ui.telemetryUpdates.length, 1);

  const changedContext = layoutContext("demo-a");
  changedContext.cells[1].rotation = 3;
  harness.setContext(changedContext);
  await waitFor(
    () =>
      harness.calls.filter((call) => call.name === "profile_activate").length ===
      2,
  );
  harness.emitTauri("scrapmap:telemetry", {
    sequence: 11,
    worldId: "demo-a",
    player: { worldId: "demo-a", x: 111, y: 111, z: 0 },
  });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(harness.ui.telemetryUpdates.length, 1);

  const activationRequest = harness.calls.filter(
    (call) => call.name === "profile_activate",
  )[1].args.request;
  secondActivation.resolve(activatedSnapshot(activationRequest));
  await waitFor(() => harness.body.dataset.profileState === "ready");

  harness.emitTauri("scrapmap:telemetry", {
    sequence: 1,
    worldId: "demo-a",
    player: { worldId: "demo-a", x: 1, y: 2, z: 0 },
  });
  await waitFor(() => harness.ui.telemetryUpdates.length === 2);
  assert.equal(harness.ui.telemetry.player.x, 1);

  const accepted = harness.calls.filter(
    (call) => call.name === "profile_accept_telemetry_sequence",
  );
  assert.equal(accepted.length, 2);
  assert.notEqual(
    accepted[0].args.request.context.sessionId,
    accepted.at(-1).args.request.context.sessionId,
  );
});

test("a fog mutation arriving during an IPC write is flushed afterwards", async () => {
  const firstFogWrite = deferred();
  let fogWrites = 0;
  const harness = createHarness();
  await waitFor(() => harness.body.dataset.profileState === "ready");

  harness.handlers.mergeFog = () => {
    fogWrites += 1;
    return fogWrites === 1
      ? firstFogWrite.promise
      : { inserted: 1, total: fogWrites };
  };

  harness.reveal([{ x: 0, y: 0 }]);
  await waitFor(() => fogWrites === 1);
  harness.reveal([{ x: 1, y: 0 }]);
  firstFogWrite.resolve({ inserted: 1, total: 1 });

  await waitFor(() => fogWrites === 2);
  const writes = harness.calls.filter(
    (call) => call.name === "profile_merge_fog",
  );
  assert.deepEqual(writes[0].args.request.cells, [{ x: 0, y: 0 }]);
  assert.deepEqual(writes[1].args.request.cells, [{ x: 1, y: 0 }]);
});

test("legacy local Tauri state is imported only after successful SQLite writes", async () => {
  const sourceWorldId = "demo-a";
  const marker = {
    id: "local-legacy",
    cellX: 1,
    cellY: 0,
    kind: "x",
    label: "Старая метка",
    createdAt: "2026-07-29T12:00:00Z",
  };
  const harness = createHarness({
    context: layoutContext(sourceWorldId),
    localStorage: {
      [`sm-minimap:visited:${sourceWorldId}`]: JSON.stringify({
        schemaVersion: 1,
        worldId: sourceWorldId,
        visited: ["1,0"],
      }),
      [`sm-minimap:markers:${sourceWorldId}`]: JSON.stringify({
        schemaVersion: 1,
        worldId: sourceWorldId,
        markers: [marker],
      }),
      [`sm-minimap:poi-filters:${sourceWorldId}`]: JSON.stringify({
        schemaVersion: 1,
        worldId: sourceWorldId,
        enabled: ["warehouse"],
        fogEnabled: false,
      }),
    },
  });

  await waitFor(() => harness.body.dataset.profileState === "ready");
  const fogWrite = harness.calls.find(
    (call) => call.name === "profile_merge_fog",
  );
  const markerWrite = harness.calls.find(
    (call) => call.name === "profile_replace_local_markers",
  );
  const settingsWrite = harness.calls.find(
    (call) => call.name === "profile_save_settings",
  );
  assert.deepEqual(fogWrite.args.request.cells, [{ x: 1, y: 0 }]);
  assert.deepEqual(markerWrite.args.request.markers, [marker]);
  assert.deepEqual(settingsWrite.args.request.settings.poiEnabled, [
    "warehouse",
  ]);
  assert.equal(settingsWrite.args.request.settings.fogEnabled, false);

  const migrationKey = [...harness.localStorageEntries.keys()].find((key) =>
    key.startsWith("sm-minimap:sqlite-imported:"),
  );
  assert.ok(migrationKey);
});

test("failed legacy import remains retryable", async () => {
  const sourceWorldId = "demo-a";
  const harness = createHarness({
    context: layoutContext(sourceWorldId),
    saveSettings() {
      throw new Error("simulated SQLite write failure");
    },
    console: { error() {}, warn() {}, log() {} },
    localStorage: {
      [`sm-minimap:poi-filters:${sourceWorldId}`]: JSON.stringify({
        schemaVersion: 1,
        worldId: sourceWorldId,
        enabled: ["warehouse"],
        fogEnabled: false,
      }),
    },
  });

  await waitFor(() => harness.body.dataset.profileState === "error");
  assert.equal(
    [...harness.localStorageEntries.keys()].some((key) =>
      key.startsWith("sm-minimap:sqlite-imported:"),
    ),
    false,
  );
});

test("unknown remote profile stays read-only until a named manual profile is selected", async () => {
  const candidates = [];
  const harness = createHarness({
    context: layoutContext("survival-remote"),
    diagnosticActive: true,
    activate(request) {
      const snapshot = remoteSnapshot(request);
      if (request.fallbackProfileId) {
        candidates.splice(
          0,
          candidates.length,
          {
            schemaVersion: 1,
            profileKey: snapshot.profile.profileKey,
            worldFingerprint: snapshot.profile.worldFingerprint,
            fallbackProfileId: request.fallbackProfileId,
            displayName: request.fallbackProfileName,
            lastOpenedAtMs: Date.now(),
          },
        );
      }
      return snapshot;
    },
    listManualProfiles(request) {
      return {
        schemaVersion: 1,
        worldFingerprint: request.worldFingerprint,
        candidates: structuredClone(candidates),
      };
    },
  });

  await waitFor(
    () => harness.body.dataset.profileState === "needs-selection",
  );
  harness.reveal([{ x: 1, y: 0 }]);
  harness.setActiveRoute(
    routeFixture(`smwf1_${"a".repeat(64)}`, "quarantine-route"),
  );
  harness.writeTrailBatch(trailBatchFixture("quarantine-trail"));
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(
    harness.calls.some((call) =>
      [
        "profile_merge_fog",
        "profile_replace_local_markers",
        "profile_save_settings",
        "profile_set_active_route",
        "profile_write_trail_batch",
      ].includes(call.name),
    ),
    false,
  );

  harness.profiles.create("Сервер друга");
  await waitFor(
    () =>
      harness.body.dataset.profileState === "ready" &&
      harness.calls.some((call) => call.name === "profile_merge_fog"),
  );

  const activations = harness.calls.filter(
    (call) => call.name === "profile_activate",
  );
  assert.equal(activations.length, 2);
  assert.equal(activations[0].args.request.fallbackProfileId, null);
  assert.match(
    activations[1].args.request.fallbackProfileId,
    /^manual:[0-9a-f-]+$/,
  );
  assert.equal(
    activations[1].args.request.fallbackProfileName,
    "Сервер друга",
  );
  assert.notEqual(
    activations[0].args.request.sessionId,
    activations[1].args.request.sessionId,
  );
  assert.deepEqual(
    harness.calls.find((call) => call.name === "profile_merge_fog").args
      .request.cells,
    [{ x: 1, y: 0 }],
  );
  assert.deepEqual(harness.hydrations.at(-1).visited, [{ x: 1, y: 0 }]);
  assert.equal(harness.profiles.getState().profile.displayName, "Сервер друга");

  const manualId = activations[1].args.request.fallbackProfileId;
  harness.profiles.select(manualId);
  await waitFor(
    () =>
      harness.calls.filter((call) => call.name === "profile_activate").length ===
        3 && harness.body.dataset.profileState === "ready",
  );
  const selectedAgain = harness.calls.filter(
    (call) => call.name === "profile_activate",
  )[2].args.request;
  assert.equal(selectedAgain.fallbackProfileId, manualId);
  assert.notEqual(selectedAgain.sessionId, activations[1].args.request.sessionId);
});

test("opaque peer identity from the native log probe activates a stable server profile", async () => {
  const stableId = `steam-sha256:${"b".repeat(64)}`;
  const harness = createHarness({
    context: layoutContext("survival-peer"),
    diagnosticActive: true,
    serverIdentityProbe: {
      identity: { kind: "peer-hosted", stableId },
      observationId: `connection-sha256:${"c".repeat(64)}`,
      issue: null,
    },
    activate(request) {
      const snapshot = remoteSnapshot(request);
      snapshot.profile = {
        ...snapshot.profile,
        profileKey: "smp1-stable-peer",
        scopeKind: "server",
        scopeId: `peer-hosted:${stableId}`,
        identityQuality: "stable",
        serverKind: "peer-hosted",
        serverStableId: stableId,
        needsManualDisambiguation: false,
      };
      return snapshot;
    },
  });

  await waitFor(() => harness.body.dataset.profileState === "ready");
  const activation = harness.calls.find(
    (call) => call.name === "profile_activate",
  );
  assert.deepEqual(activation.args.request.server, {
    kind: "peer-hosted",
    stableId,
  });
  assert.equal(harness.body.dataset.profileNeedsSplit, "false");
  assert.equal(
    harness.calls.some(
      (call) => call.name === "profile_list_manual_profiles",
    ),
    false,
  );
});

test("a reused connection observation cannot identify a new automatic activation", async () => {
  const stableId = `steam-sha256:${"b".repeat(64)}`;
  const observationId = `connection-sha256:${"c".repeat(64)}`;
  const harness = createHarness({
    context: layoutContext("survival-peer"),
    diagnosticActive: true,
    serverIdentityProbe: {
      identity: { kind: "peer-hosted", stableId },
      observationId,
      issue: null,
    },
    activate(request) {
      if (request.server.kind === "unknown") {
        return remoteSnapshot(request);
      }
      const snapshot = remoteSnapshot(request);
      snapshot.profile = {
        ...snapshot.profile,
        profileKey: "smp1-stable-peer",
        scopeKind: "server",
        scopeId: `peer-hosted:${stableId}`,
        identityQuality: "stable",
        serverKind: "peer-hosted",
        serverStableId: stableId,
        needsManualDisambiguation: false,
      };
      return snapshot;
    },
  });
  await waitFor(() => harness.body.dataset.profileState === "ready");

  const changedContext = layoutContext("survival-peer");
  changedContext.cells[1].rotation = 3;
  harness.setContext(changedContext);
  await waitFor(
    () => harness.body.dataset.profileState === "needs-selection",
  );

  const activations = harness.calls.filter(
    (call) => call.name === "profile_activate",
  );
  assert.equal(activations.length, 2);
  assert.deepEqual(activations[0].args.request.server, {
    kind: "peer-hosted",
    stableId,
  });
  assert.deepEqual(activations[1].args.request.server, {
    kind: "unknown",
    stableId: null,
  });
});

test("a new connection observation reactivates the same layout with a fresh session", async () => {
  const stableId = `steam-sha256:${"b".repeat(64)}`;
  let observationId = `connection-sha256:${"c".repeat(64)}`;
  const harness = createHarness({
    context: layoutContext("survival-peer"),
    diagnosticActive: true,
    serverIdentityProbe() {
      return {
        identity: { kind: "peer-hosted", stableId },
        observationId,
        issue: null,
      };
    },
    activate(request) {
      const snapshot = remoteSnapshot(request);
      snapshot.profile = {
        ...snapshot.profile,
        profileKey: "smp1-stable-peer",
        scopeKind: "server",
        scopeId: `peer-hosted:${stableId}`,
        identityQuality: "stable",
        serverKind: "peer-hosted",
        serverStableId: stableId,
        needsManualDisambiguation: false,
      };
      return snapshot;
    },
  });
  await waitFor(() => harness.body.dataset.profileState === "ready");
  observationId = `connection-sha256:${"d".repeat(64)}`;

  assert.equal(await harness.profiles.refreshConnection(), true);
  await waitFor(
    () =>
      harness.calls.filter((call) => call.name === "profile_activate").length ===
        2 && harness.body.dataset.profileState === "ready",
  );

  const activations = harness.calls.filter(
    (call) => call.name === "profile_activate",
  );
  assert.notEqual(
    activations[0].args.request.sessionId,
    activations[1].args.request.sessionId,
  );
  assert.equal(
    activations[1].args.request.layout.worldId,
    activations[0].args.request.layout.worldId,
  );
});
