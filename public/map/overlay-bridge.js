(function connectTauriOverlay() {
  "use strict";

  /** Interface string by key. Falls back to the key so a missing i18n layer
   *  degrades to something identifiable rather than to blank UI. */
  const text = (key) => window.SMText?.t(key) ?? key;

  // The tray menu is built before a WebView exists to ask, so the choice is
  // left where the native side can read it on the next start.
  window.addEventListener("sm-minimap:language", (event) => {
    const language = String(event?.detail?.language || "");
    if (language && invoke) {
      invoke("set_interface_language", { language }).catch(() => {});
    }
  });

  const tauri = window.__TAURI__;
  const invoke = tauri?.core?.invoke;
  const listen = tauri?.event?.listen;
  if (typeof invoke !== "function" || typeof listen !== "function") {
    document.body.dataset.overlayHost = "browser";
    document.body.classList.remove("scrapmap-overlay", "overlay-mini");
    const browserProfileButton = document.getElementById?.(
      "profileStatusButton"
    );
    const browserProfileSummary = document.getElementById?.("profileSummary");
    if (browserProfileButton) {
      browserProfileButton.textContent = "БРАУЗЕР";
      browserProfileButton.disabled = true;
      browserProfileButton.removeAttribute("aria-haspopup");
      browserProfileButton.removeAttribute("aria-controls");
    }
    if (browserProfileSummary) {
      browserProfileSummary.textContent = "Данные хранятся в этом браузере.";
    }
    return;
  }

  const PROFILE_SCHEMA_VERSION = 1;
  const MAX_FOG_BATCH = 4096;
  const DEFAULT_POI_CATEGORIES = [
    "schematic",
    "quest",
    "camp",
    "warehouse",
    "service",
    "dungeon",
    "landmark"
  ];

  document.body.dataset.overlayHost = "tauri";
  document.body.dataset.profileState = "idle";

  let applyingNativeMode = false;
  let telemetryRejected = false;
  let profileGeneration = 0;
  let latestProfileJob = null;
  let activeProfile = null;
  let storageQueue = Promise.resolve();
  let profileCandidates = [];
  let profileUiSnapshot = null;
  let lastConnectionObservationId = null;
  let connectionMonitorInFlight = false;
  let layoutRefreshInFlight = false;
  let appliedGameLayoutKey = null;

  const elementById = (id) => document.getElementById?.(id) || null;
  const profileElements = {
    button: elementById("profileStatusButton"),
    summary: elementById("profileSummary"),
    dialog: elementById("profileDialog"),
    backdrop: elementById("profileDialogBackdrop"),
    close: elementById("profileDialogClose"),
    copy: elementById("profileDialogCopy"),
    list: elementById("profileCandidateList"),
    form: elementById("profileCreateForm"),
    input: elementById("profileNameInput"),
    status: elementById("profileDialogStatus"),
    worldChip: elementById("worldChip")
  };

  function makeSessionId() {
    if (typeof window.crypto?.randomUUID === "function") {
      return `overlay-${window.crypto.randomUUID()}`;
    }
    const random = Math.random().toString(36).slice(2);
    return `overlay-${Date.now().toString(36)}-${random}`;
  }

  function enqueueStorage(task) {
    const operation = storageQueue
      .catch(() => undefined)
      .then(task);
    storageQueue = operation.catch((error) => {
      console.error("ScrapMap profile storage operation failed", error);
    });
    return operation;
  }

  async function setNativeMode(expanded) {
    try {
      await invoke("set_overlay_mode", { expanded: Boolean(expanded) });
    } catch (error) {
      console.error("ScrapMap overlay mode change failed", error);
      try {
        const status = await invoke("overlay_status");
        applyMode(Boolean(status?.expanded));
      } catch (statusError) {
        console.error("ScrapMap overlay mode recovery failed", statusError);
      }
    }
  }

  window.addEventListener("sm-minimap:mode-request", (event) => {
    if (!applyingNativeMode) {
      setNativeMode(Boolean(event.detail?.expanded));
    }
  });

  function applyMode(expanded) {
    applyingNativeMode = true;
    try {
      window.SMMinimap?.setExpanded(expanded);
      document.body.classList.toggle("overlay-mini", !expanded);
    } finally {
      applyingNativeMode = false;
    }
  }

  function applyTelemetry(payload) {
    if (!payload || typeof payload !== "object") {
      return;
    }

    try {
      window.SMMinimap?.updateTelemetry(payload, { live: true });
      telemetryRejected = false;
    } catch (error) {
      telemetryRejected = true;
      console.error("ScrapMap telemetry update was rejected", error);
    }
  }

  function applyDiagnosticStatus(payload) {
    if (!payload || typeof payload !== "object") {
      return;
    }

    const reportedState = String(payload.state || "waiting");
    const state =
      reportedState === "active" && telemetryRejected
        ? "invalid"
        : reportedState;
    window.SMMinimap?.setTelemetryStatus(state);
    document.body.dataset.telemetryState = state;
  }

  function applyGameWindowStatus(payload) {
    if (!payload || typeof payload !== "object") {
      return;
    }
    document.body.dataset.gameWindowState = String(payload.state || "missing");
  }

  function gameLayoutKey(layout) {
    const bounds = layout?.bounds || {};
    return [
      String(layout?.worldId || ""),
      String(layout?.seed ?? ""),
      Array.isArray(layout?.cells) ? layout.cells.length : 0,
      bounds.minX,
      bounds.maxX,
      bounds.minY,
      bounds.maxY
    ].join("|");
  }

  async function refreshGameLayout() {
    if (layoutRefreshInFlight) return false;
    layoutRefreshInFlight = true;
    try {
      const layout = await invoke("game_layout_snapshot");
      if (!layout || !Array.isArray(layout.cells) || !layout.cells.length) {
        return false;
      }
      const key = gameLayoutKey(layout);
      if (!layout.worldId || key === appliedGameLayoutKey) {
        return false;
      }
      window.SMMinimap?.setLayout(layout);
      appliedGameLayoutKey = key;
      document.body.dataset.gameLayout = "active";
      return true;
    } catch (error) {
      document.body.dataset.gameLayout = "unavailable";
      if (typeof console.warn === "function") {
        console.warn("ScrapMap game layout is not ready", error);
      }
      return false;
    } finally {
      layoutRefreshInFlight = false;
    }
  }

  // Converts whatever the in-game baker has produced into map tiles. Returns
  // how many tiles were newly converted, so callers can tell when the manifest
  // is worth re-reading.
  async function refreshGeneratedTiles() {
    try {
      const report = await invoke("atlas_bake_refresh");
      return Number(report?.converted || 0);
    } catch (error) {
      console.info("ScrapMap generated tile conversion is unavailable", error);
      return 0;
    }
  }

  async function bootstrapTileAtlas() {
    if (typeof window.SMMinimap?.setTileAtlas !== "function") {
      return;
    }

    try {
      await refreshGeneratedTiles();
      const manifest = await invoke("atlas_manifest");
      if (!manifest || !Array.isArray(manifest.entries) || !manifest.entries.length) {
        return;
      }

      const status = window.SMMinimap.setTileAtlas(manifest, {
        resolveSource: (entry) =>
          invoke("atlas_preview", {
            request: {
              relativePath: entry.topDownRelativePath || entry.relativePath
            }
          })
      });
      document.body.dataset.tileAtlas = status?.contentFingerprint || "ready";
    } catch (error) {
      document.body.dataset.tileAtlas = "unavailable";
      console.info("ScrapMap local tile atlas is unavailable", error);
    }
  }

  function boundedText(value, maximum, fallback = "") {
    const text = String(value == null ? fallback : value).trim();
    return text.slice(0, maximum);
  }

  function requiredBoundedText(name, value, maximum) {
    const text = String(value == null ? "" : value).trim();
    if (!text || text.length > maximum) {
      throw new TypeError(`${name} must contain 1..${maximum} characters.`);
    }
    return text;
  }

  function isManualProfileId(value) {
    return /^manual:[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(
      String(value || "")
    );
  }

  function finiteNumber(value, fallback = 0) {
    const number = Number(value);
    return Number.isFinite(number) ? number : fallback;
  }

  function integer(value, fallback = 0) {
    const number = Number(value);
    return Number.isInteger(number) ? number : fallback;
  }

  function setProfileDialogStatus(message, isError = false) {
    if (!profileElements.status) return;
    profileElements.status.textContent = String(message || "");
    profileElements.status.classList?.toggle("is-error", isError);
  }

  function profilePresentation(snapshot, state) {
    const profile = snapshot?.profile || {};
    if (state === "loading" || state === "switching") {
      return {
        badge: text(state === "switching" ? "BADGE_SWITCHING" : "BADGE_RESOLVING"),
        summary: text("SESSION_RESOLVING_SUMMARY")
      };
    }
    if (state === "error") {
      return {
        badge: text("BADGE_ERROR"),
        summary: text("SUMMARY_ERROR")
      };
    }
    if (profile.needsManualDisambiguation === true) {
      return {
        badge: text("BADGE_SERVER_UNKNOWN"),
        summary: text("SUMMARY_SERVER_UNKNOWN")
      };
    }
    if (profile.scopeKind === "local") {
      return { badge: text("BADGE_LOCAL"), summary: text("SUMMARY_LOCAL") };
    }
    if (profile.identityQuality === "manual") {
      return {
        badge: text("BADGE_MANUAL"),
        summary:
          boundedText(profile.displayName, 80) || text("SUMMARY_MANUAL")
      };
    }
    return { badge: text("BADGE_SERVER"), summary: text("SUMMARY_SERVER") };
  }

  function renderProfileCandidates(snapshot) {
    const list = profileElements.list;
    if (!list || typeof document.createElement !== "function") return;
    list.replaceChildren();
    if (!profileCandidates.length) {
      const empty = document.createElement("p");
      empty.className = "profile-candidate-empty";
      empty.textContent = "Именованных профилей для этой карты пока нет.";
      list.appendChild(empty);
      return;
    }

    for (const candidate of profileCandidates) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "profile-candidate";
      button.dataset.fallbackProfileId = candidate.fallbackProfileId;
      if (candidate.profileKey === snapshot?.profile?.profileKey) {
        button.classList.add("is-active");
      }

      const text = document.createElement("span");
      const name = document.createElement("strong");
      name.textContent =
        boundedText(candidate.displayName, 80, "") || "Ручной профиль";
      const opened = document.createElement("small");
      const date = new Date(Number(candidate.lastOpenedAtMs));
      opened.textContent = Number.isFinite(date.getTime())
        ? `Открывался ${date.toLocaleDateString("ru-RU")}`
        : "Сохранённый профиль";
      text.append(name, opened);

      const action = document.createElement("b");
      action.textContent =
        candidate.profileKey === snapshot?.profile?.profileKey
          ? "АКТИВЕН"
          : "ВЫБРАТЬ";
      button.append(text, action);
      button.addEventListener("click", () => {
        window.ScrapMapProfiles?.select(candidate.fallbackProfileId);
      });
      list.appendChild(button);
    }
  }

  function renderProfileUi(snapshot, state) {
    profileUiSnapshot = snapshot || profileUiSnapshot;
    const presentation = profilePresentation(profileUiSnapshot, state);
    if (profileElements.button) {
      profileElements.button.textContent = presentation.badge;
      profileElements.button.title = presentation.summary;
    }
    if (profileElements.summary) {
      profileElements.summary.textContent = presentation.summary;
    }
    if (profileElements.worldChip && profileUiSnapshot?.profile) {
      profileElements.worldChip.textContent = presentation.summary;
      profileElements.worldChip.title =
        latestProfileJob?.layout?.sourceWorldId || "";
    }

    const manualAllowed =
      profileUiSnapshot?.profile?.scopeKind === "fallback" ||
      profileUiSnapshot?.profile?.needsManualDisambiguation === true;
    if (profileElements.form) {
      profileElements.form.hidden = !manualAllowed;
    }
    if (profileElements.copy) {
      profileElements.copy.textContent = manualAllowed
        ? "Идентификатор сервера недоступен. Выберите профиль, чтобы туман и метки одинаковых карт не смешались."
        : "Этот профиль распознан автоматически; ручной выбор не требуется.";
    }
    renderProfileCandidates(profileUiSnapshot);
  }

  function openProfileDialog() {
    if (!profileElements.dialog) return;
    profileElements.dialog.classList.add("is-open");
    profileElements.dialog.setAttribute("aria-hidden", "false");
    setProfileDialogStatus("");
    const target =
      profileElements.input && !profileElements.form?.hidden
        ? profileElements.input
        : profileElements.close;
    target?.focus?.();
  }

  function closeProfileDialog() {
    if (!profileElements.dialog) return;
    profileElements.dialog.classList.remove("is-open");
    profileElements.dialog.setAttribute("aria-hidden", "true");
    profileElements.button?.focus?.();
  }

  function arrayFromPayload(payload, keys = []) {
    if (Array.isArray(payload)) {
      return payload;
    }
    if (!payload || typeof payload !== "object") {
      return [];
    }
    for (const key of keys) {
      if (Array.isArray(payload[key])) {
        return payload[key];
      }
    }
    return [];
  }

  function normalizeLayoutContext(value) {
    if (!value || typeof value !== "object" || !Array.isArray(value.cells)) {
      throw new TypeError("Profile context must contain layout cells.");
    }

    const sourceWorldId = requiredBoundedText(
      "Profile sourceWorldId",
      value.sourceWorldId,
      160
    );
    if (!value.bounds || typeof value.bounds !== "object") {
      throw new TypeError("Profile context is missing layout bounds.");
    }

    return {
      schemaVersion: PROFILE_SCHEMA_VERSION,
      sourceWorldId,
      gameMode: boundedText(value.gameMode, 32, "unknown") || "unknown",
      cellSize: finiteNumber(value.cellSize),
      bounds: {
        minX: integer(value.bounds.minX),
        maxX: integer(value.bounds.maxX),
        minY: integer(value.bounds.minY),
        maxY: integer(value.bounds.maxY)
      },
      cells: value.cells.map((cell) => ({
        x: integer(cell.x),
        y: integer(cell.y),
        tileUuid: requiredBoundedText("Tile identity", cell.tileUuid, 512),
        rotation: integer(cell.rotation),
        xOffset: finiteNumber(cell.xOffset),
        yOffset: finiteNumber(cell.yOffset),
        // Multi-cell tiles share one atlas image; the renderer needs the size
        // to pick this cell's slice out of it.
        tileSize: finiteNumber(cell.tileSize),
        flags: integer(cell.flags)
      }))
    };
  }

  function mutationMatchesJob(detail, job) {
    return Boolean(
      detail &&
      job &&
      String(detail.sourceWorldId || "").trim() === job.layout.sourceWorldId
    );
  }

  function addFogToJob(job, cells) {
    for (const cell of cells || []) {
      const x = Number(cell?.x);
      const y = Number(cell?.y);
      const key = `${x},${y}`;
      if (
        Number.isInteger(x) &&
        Number.isInteger(y) &&
        (!job.validCells || job.validCells.has(key))
      ) {
        job.fog.set(key, { x, y });
      }
    }
  }

  function replaceMarkersInJob(job, markers) {
    job.markers = Array.isArray(markers)
      ? markers.map((marker) => ({ ...marker }))
      : [];
  }

  function replaceSettingsInJob(job, settings) {
    if (settings && typeof settings === "object") {
      job.settings = {
        poiCategories: Array.isArray(settings.poiCategories)
          ? settings.poiCategories.map(String)
          : [],
        fogEnabled: settings.fogEnabled !== false
      };
    }
  }

  function writeContextFromSnapshot(snapshot, fallbackSessionId) {
    const context = snapshot?.context;
    if (
      context &&
      typeof context.profileKey === "string" &&
      typeof context.worldFingerprint === "string" &&
      typeof context.sessionId === "string"
    ) {
      return context;
    }

    const profile = snapshot?.profile;
    if (
      profile &&
      typeof profile.profileKey === "string" &&
      typeof profile.worldFingerprint === "string"
    ) {
      return {
        profileKey: profile.profileKey,
        worldFingerprint: profile.worldFingerprint,
        sessionId: boundedText(snapshot.sessionId, 128, fallbackSessionId)
      };
    }
    throw new TypeError("Profile activation returned no write context.");
  }

  function profileHydrationPayload(snapshot, job) {
    const profile = snapshot?.profile || {};
    const visitedPayload = snapshot?.visited || snapshot?.revealedCells || [];
    const localMarkersPayload = snapshot?.localMarkers || snapshot?.markers || [];
    const sharedMarkersPayload = snapshot?.sharedMarkers || [];
    const worldFingerprint = String(profile.worldFingerprint || "");
    for (const payload of [snapshot?.visited, snapshot?.markers]) {
      if (
        payload &&
        !Array.isArray(payload) &&
        payload.worldId &&
        String(payload.worldId) !== worldFingerprint
      ) {
        throw new TypeError("Profile snapshot contains mixed world identities.");
      }
    }
    const settings =
      snapshot?.settings && typeof snapshot.settings === "object"
        ? snapshot.settings
        : {};
    const hydrationSettings = { ...settings };
    let poiCategories = null;
    if (Array.isArray(settings.poiCategories)) {
      poiCategories = settings.poiCategories;
    } else if (Array.isArray(settings.poiEnabled)) {
      poiCategories = settings.poiEnabled;
    }
    if (poiCategories) {
      hydrationSettings.poiCategories = poiCategories;
    } else {
      delete hydrationSettings.poiCategories;
    }

    return {
      schemaVersion: PROFILE_SCHEMA_VERSION,
      profileKey: profile.profileKey,
      worldFingerprint,
      sourceWorldId: job.layout.sourceWorldId,
      visited: arrayFromPayload(visitedPayload, ["visited", "cells"]),
      localMarkers: arrayFromPayload(localMarkersPayload, ["markers"]),
      sharedMarkers: arrayFromPayload(sharedMarkersPayload, ["markers"]),
      settings: hydrationSettings,
      activeRoute:
        snapshot?.activeRoute && typeof snapshot.activeRoute === "object"
          ? snapshot.activeRoute
          : null,
      recentTrail:
        snapshot?.recentTrail && typeof snapshot.recentTrail === "object"
          ? snapshot.recentTrail
          : null
    };
  }

  function cellsFromRendererSnapshot(snapshot) {
    const result = [];
    for (const value of snapshot?.visited || []) {
      if (typeof value === "string") {
        const match = /^(-?\d+),(-?\d+)$/.exec(value);
        if (match) {
          result.push({ x: Number(match[1]), y: Number(match[2]) });
        }
      } else if (value && Number.isInteger(value.x) && Number.isInteger(value.y)) {
        result.push({ x: value.x, y: value.y });
      }
    }
    return result;
  }

  function legacyVisitedCells(payload) {
    const cells = [];
    for (const entry of arrayFromPayload(payload, ["visited"])) {
      if (typeof entry === "string") {
        const match = /^(-?\d+),(-?\d+)$/.exec(entry);
        if (match) {
          cells.push({ x: Number(match[1]), y: Number(match[2]) });
        }
        continue;
      }
      const x = Number(entry?.x ?? entry?.cellX);
      const y = Number(entry?.y ?? entry?.cellY);
      if (Number.isInteger(x) && Number.isInteger(y)) {
        cells.push({ x, y });
      }
    }
    return cells;
  }

  function legacyMarkers(payload, job) {
    const markers = new Map();
    for (const entry of arrayFromPayload(payload, ["markers"])) {
      const id = String(entry?.id || "").trim();
      const x = Number(entry?.cellX ?? entry?.x);
      const y = Number(entry?.cellY ?? entry?.y);
      const label = String(entry?.label || "").trim();
      if (
        !id ||
        id.length > 160 ||
        !Number.isInteger(x) ||
        !Number.isInteger(y) ||
        !job.validCells.has(`${x},${y}`) ||
        !label ||
        [...label].length > 160
      ) {
        continue;
      }
      markers.set(id, {
        id,
        cellX: x,
        cellY: y,
        kind: boundedText(entry?.kind, 64, "x") || "x",
        label,
        createdAt:
          typeof entry?.createdAt === "string"
            ? entry.createdAt.slice(0, 160)
            : null
      });
    }
    return Array.from(markers.values());
  }

  function readLegacyLocalProfile(snapshot, job) {
    if (
      snapshot?.profile?.scopeKind !== "local" ||
      typeof window.localStorage?.getItem !== "function"
    ) {
      return null;
    }

    const profileKey = String(snapshot.profile.profileKey || "");
    const migrationKey = `sm-minimap:sqlite-imported:${profileKey}`;
    try {
      if (window.localStorage.getItem(migrationKey)) {
        return null;
      }

      const sourceWorldId = job.layout.sourceWorldId;
      const visitedText = window.localStorage.getItem(
        `sm-minimap:visited:${sourceWorldId}`
      );
      const markersText = window.localStorage.getItem(
        `sm-minimap:markers:${sourceWorldId}`
      );
      const settingsText = window.localStorage.getItem(
        `sm-minimap:poi-filters:${sourceWorldId}`
      );
      const parse = (text) => (text ? JSON.parse(text) : null);
      const settings = parse(settingsText);
      const storedFog = arrayFromPayload(snapshot?.visited, ["visited"]);
      const storedMarkers = arrayFromPayload(snapshot?.markers, ["markers"]);
      const storedPoi = Array.isArray(snapshot?.settings?.poiEnabled)
        ? snapshot.settings.poiEnabled
        : [];
      const settingsAreDefault =
        snapshot?.settings?.fogEnabled !== false &&
        storedPoi.length === DEFAULT_POI_CATEGORIES.length &&
        DEFAULT_POI_CATEGORIES.every((category) =>
          storedPoi.includes(category)
        );
      const fog =
        storedFog.length === 0
          ? legacyVisitedCells(parse(visitedText))
          : [];
      const markers =
        storedMarkers.length === 0 && markersText
          ? legacyMarkers(parse(markersText), job)
          : null;
      let profileSettings = null;
      if (
        settingsAreDefault &&
        settings &&
        Array.isArray(settings.enabled)
      ) {
        profileSettings = {
          poiCategories: settings.enabled.map(String),
          fogEnabled: settings.fogEnabled !== false
        };
      }
      return {
        migrationKey,
        fog,
        markers,
        settings: profileSettings
      };
    } catch (error) {
      console.warn("ScrapMap could not read legacy local profile data", error);
      return null;
    }
  }

  function completeLegacyMigration(job) {
    if (
      !job.legacyMigrationKey ||
      typeof window.localStorage?.setItem !== "function"
    ) {
      return;
    }
    try {
      window.localStorage.setItem(
        job.legacyMigrationKey,
        JSON.stringify({
          schemaVersion: 1,
          importedAt: new Date().toISOString()
        })
      );
      job.legacyMigrationKey = null;
    } catch (error) {
      console.warn("ScrapMap could not mark legacy profile migration complete", error);
    }
  }

  function captureCurrentRendererState(job) {
    const snapshot = window.SMMinimap?.getSnapshot();
    if (!snapshot || snapshot.worldId !== job.layout.sourceWorldId) {
      return;
    }

    addFogToJob(job, cellsFromRendererSnapshot(snapshot));
    if (job.markers === null) {
      replaceMarkersInJob(
        job,
        (snapshot.markers || []).filter((marker) => marker.local === true)
      );
    }
    if (job.settings === null) {
      replaceSettingsInJob(job, {
        poiCategories: snapshot.poiCategories,
        fogEnabled: snapshot.fogEnabled
      });
    }
  }

  function delay(milliseconds) {
    return new Promise((resolve) => window.setTimeout(resolve, milliseconds));
  }

  function telemetryMatchesLayout(telemetry, sourceWorldId) {
    const worldIds = [
      telemetry?.worldId,
      telemetry?.payloadWorldId,
      telemetry?.player?.worldId,
      telemetry?.player?.payloadWorldId
    ]
      .map((value) => String(value || ""))
      .filter(Boolean);
    return (
      worldIds.length > 0 &&
      worldIds.every((worldId) => worldId === sourceWorldId)
    );
  }

  function telemetrySequence(telemetry) {
    const sequence = Number(telemetry?.sequence);
    return Number.isSafeInteger(sequence) && sequence > 0 ? sequence : null;
  }

  function acceptTelemetryForActiveProfile(telemetry) {
    const sequence = telemetrySequence(telemetry);
    const job = activeProfile?.job;
    if (
      sequence === null ||
      !job ||
      latestProfileJob !== job ||
      !telemetryMatchesLayout(telemetry, job.layout.sourceWorldId) ||
      sequence <= job.lastTelemetrySequence
    ) {
      if (sequence === null) {
        telemetryRejected = true;
      }
      return;
    }

    enqueueStorage(async () => {
      const accepted = await invoke("profile_accept_telemetry_sequence", {
        request: {
          schemaVersion: PROFILE_SCHEMA_VERSION,
          context: job.writeContext,
          sequence
        }
      });
      if (
        accepted !== true ||
        activeProfile?.job !== job ||
        latestProfileJob !== job ||
        job.generation !== profileGeneration ||
        sequence <= job.lastTelemetrySequence
      ) {
        return;
      }
      job.lastTelemetrySequence = sequence;
      applyTelemetry(telemetry);
    });
  }

  function receiveTelemetry(telemetry) {
    const sequence = telemetrySequence(telemetry);
    const job = latestProfileJob;
    if (
      !activeProfile &&
      job?.acceptPendingInteractiveMutations &&
      sequence !== null &&
      telemetryMatchesLayout(telemetry, job.layout.sourceWorldId)
    ) {
      const pendingSequence = telemetrySequence(job.pendingTelemetry);
      if (pendingSequence === null || sequence > pendingSequence) {
        job.pendingTelemetry = telemetry;
      }
      return;
    }
    acceptTelemetryForActiveProfile(telemetry);
  }

  async function nativeServerIdentity() {
    const probe = await invoke("server_identity_probe");
    const identity = probe?.identity;
    const observationId = String(probe?.observationId || "");
    if (!/^connection-sha256:[0-9a-f]{64}$/.test(observationId)) {
      return null;
    }
    if (identity?.kind === "local") {
      return { kind: "local", stableId: null, observationId };
    }
    if (
      identity?.kind === "peer-hosted" &&
      /^steam-sha256:[0-9a-f]{64}$/.test(String(identity.stableId || ""))
    ) {
      return {
        kind: "peer-hosted",
        stableId: identity.stableId,
        observationId
      };
    }
    return null;
  }

  async function refreshConnectionObservation() {
    const job = activeProfile?.job;
    if (
      connectionMonitorInFlight ||
      !job?.connectionObservationId ||
      latestProfileJob !== job ||
      document.body.dataset.profileState !== "ready"
    ) {
      return false;
    }

    connectionMonitorInFlight = true;
    try {
      const observed = await nativeServerIdentity();
      if (
        activeProfile?.job !== job ||
        latestProfileJob !== job ||
        !observed?.observationId ||
        observed.observationId === job.connectionObservationId
      ) {
        return false;
      }
      lastConnectionObservationId = observed.observationId;
      requestProfileActivation(job.layout);
      return true;
    } catch (error) {
      console.warn("ScrapMap connection observation refresh failed", error);
      return false;
    } finally {
      connectionMonitorInFlight = false;
    }
  }

  async function serverIdentityForProfile(job) {
    if (job.layout.sourceWorldId.startsWith("demo-")) {
      return { kind: "local", stableId: null };
    }

    let confirmedObservationId = null;
    let confirmationCount = 0;
    for (let attempt = 0; attempt < 12; attempt += 1) {
      if (latestProfileJob !== job) {
        return { kind: "unknown", stableId: null };
      }
      try {
        const [nativeIdentity, status, telemetry] = await Promise.all([
          nativeServerIdentity(),
          invoke("diagnostic_status"),
          invoke("diagnostic_snapshot")
        ]);
        if (nativeIdentity) {
          lastConnectionObservationId = nativeIdentity.observationId;
        }
        if (
          status?.state === "active" &&
          telemetryMatchesLayout(telemetry, job.layout.sourceWorldId)
        ) {
          const expectedKind =
            telemetry?.source?.isHost === true ? "local" : "peer-hosted";
          const observationIsFresh =
            !job.connectionObservationBaseline ||
            nativeIdentity?.observationId !==
              job.connectionObservationBaseline;
          if (
            nativeIdentity?.kind === expectedKind &&
            observationIsFresh
          ) {
            if (confirmedObservationId === nativeIdentity.observationId) {
              confirmationCount += 1;
            } else {
              confirmedObservationId = nativeIdentity.observationId;
              confirmationCount = 1;
            }
            if (confirmationCount >= 5) {
              job.connectionObservationId = nativeIdentity.observationId;
              return {
                kind: nativeIdentity.kind,
                stableId: nativeIdentity.stableId
              };
            }
          } else {
            confirmedObservationId = null;
            confirmationCount = 0;
          }
        }
      } catch (error) {
        if (attempt === 11) {
          console.warn(
            "ScrapMap could not infer local/server profile identity",
            error
          );
        }
      }
      if (attempt < 11) {
        await delay(125);
      }
    }
    return { kind: "unknown", stableId: null };
  }

  function validateActivatedSnapshot(snapshot, job) {
    const profile = snapshot?.profile;
    if (
      !profile ||
      String(snapshot.sessionId || "") !== job.sessionId ||
      !String(profile.profileKey || "") ||
      !String(profile.worldFingerprint || "")
    ) {
      throw new TypeError("Profile activation returned an invalid session.");
    }
    if (
      job.expectedWorldFingerprint &&
      profile.worldFingerprint !== job.expectedWorldFingerprint
    ) {
      throw new TypeError("Selected profile belongs to another world.");
    }
    if (
      job.fallbackProfileId &&
      (profile.scopeKind !== "fallback" ||
        profile.scopeId !== job.fallbackProfileId ||
        profile.identityQuality !== "manual" ||
        profile.needsManualDisambiguation === true)
    ) {
      throw new TypeError("Native storage activated a different manual profile.");
    }
  }

  async function loadManualProfileCandidates(snapshot, job) {
    const profile = snapshot?.profile;
    if (!profile || profile.scopeKind !== "fallback") {
      return [];
    }
    const result = await invoke("profile_list_manual_profiles", {
      request: {
        schemaVersion: PROFILE_SCHEMA_VERSION,
        worldFingerprint: profile.worldFingerprint,
        serverKind: profile.serverKind || job.serverIdentity?.kind || "unknown"
      }
    });
    if (
      result?.schemaVersion !== PROFILE_SCHEMA_VERSION ||
      result?.worldFingerprint !== profile.worldFingerprint ||
      !Array.isArray(result?.candidates)
    ) {
      throw new TypeError("Manual profile list does not match the active world.");
    }

    const unique = new Map();
    for (const candidate of result.candidates.slice(0, 64)) {
      const profileKey = boundedText(candidate?.profileKey, 160);
      const worldFingerprint = boundedText(candidate?.worldFingerprint, 80);
      const fallbackProfileId = boundedText(candidate?.fallbackProfileId, 160);
      if (
        !profileKey ||
        worldFingerprint !== profile.worldFingerprint ||
        !isManualProfileId(fallbackProfileId)
      ) {
        continue;
      }
      unique.set(fallbackProfileId, {
        schemaVersion: PROFILE_SCHEMA_VERSION,
        profileKey,
        worldFingerprint,
        fallbackProfileId,
        displayName: boundedText(candidate?.displayName, 80) || null,
        lastOpenedAtMs: finiteNumber(candidate?.lastOpenedAtMs)
      });
    }
    return Array.from(unique.values());
  }

  async function flushProfileJob(job) {
    const context = job.writeContext;
    if (!context) {
      return;
    }

    const fogEntries = Array.from(job.fog.entries());
    for (let index = 0; index < fogEntries.length; index += MAX_FOG_BATCH) {
      const batch = fogEntries.slice(index, index + MAX_FOG_BATCH);
      await invoke("profile_merge_fog", {
        request: {
          schemaVersion: PROFILE_SCHEMA_VERSION,
          context,
          origin: "local",
          cells: batch.map(([, cell]) => cell)
        }
      });
      for (const [key] of batch) {
        job.fog.delete(key);
      }
    }

    if (job.markers !== null) {
      const markers = job.markers;
      await invoke("profile_replace_local_markers", {
        request: {
          schemaVersion: PROFILE_SCHEMA_VERSION,
          context,
          markers
        }
      });
      if (job.markers === markers) {
        job.markers = null;
      }
    }

    if (job.settings !== null) {
      const settings = job.settings;
      await invoke("profile_save_settings", {
        request: {
          schemaVersion: PROFILE_SCHEMA_VERSION,
          context,
          settings: {
            schemaVersion: PROFILE_SCHEMA_VERSION,
            fogEnabled: settings.fogEnabled,
            poiEnabled: settings.poiCategories
          }
        }
      });
      if (job.settings === settings) {
        job.settings = null;
      }
    }

    if (job.activeRoute !== undefined) {
      const route = job.activeRoute;
      await invoke("profile_set_active_route", {
        request: {
          schemaVersion: PROFILE_SCHEMA_VERSION,
          context,
          route
        }
      });
      if (job.activeRoute === route) {
        job.activeRoute = undefined;
      }
    }

    while (job.trailBatches.length) {
      const batch = job.trailBatches[0];
      await invoke("profile_write_trail_batch", {
        request: {
          ...batch,
          schemaVersion: PROFILE_SCHEMA_VERSION,
          context
        }
      });
      if (job.trailBatches[0] === batch) {
        job.trailBatches.shift();
      }
    }
  }

  async function activateProfileJob(job) {
    try {
      const server = job.serverIdentity || (await serverIdentityForProfile(job));
      job.serverIdentity = server;
      const snapshot = await invoke("profile_activate", {
        request: {
          schemaVersion: PROFILE_SCHEMA_VERSION,
          sessionId: job.sessionId,
          gameMode: job.layout.gameMode,
          server,
          fallbackProfileId: job.fallbackProfileId,
          fallbackProfileName: job.fallbackProfileName,
          layout: {
            schemaVersion: PROFILE_SCHEMA_VERSION,
            worldId: job.layout.sourceWorldId,
            cellSize: job.layout.cellSize,
            bounds: job.layout.bounds,
            cells: job.layout.cells
          }
        }
      });
      validateActivatedSnapshot(snapshot, job);
      job.writeContext = writeContextFromSnapshot(snapshot, job.sessionId);
      job.snapshot = snapshot;
      const needsSelection =
        snapshot?.profile?.needsManualDisambiguation === true;
      const legacy = needsSelection ? null : readLegacyLocalProfile(snapshot, job);
      if (legacy) {
        job.legacyMigrationKey = legacy.migrationKey;
        addFogToJob(job, legacy.fog);
        if (job.markers === null && legacy.markers !== null) {
          job.markers = legacy.markers;
        }
        if (job.settings === null && legacy.settings !== null) {
          job.settings = legacy.settings;
        }
      }

      let isCurrent =
        latestProfileJob === job &&
        job.generation === profileGeneration &&
        window.SMMinimap?.getProfileContext?.()?.sourceWorldId ===
          job.layout.sourceWorldId;

      if (isCurrent) {
        const hydration = profileHydrationPayload(snapshot, job);
        if (needsSelection) {
          window.SMMinimap?.setMarkerMode?.(false);
          hydration.visited = Array.from(job.fog.values());
          hydration.localMarkers = [];
          hydration.sharedMarkers = [];
          hydration.settings = {
            schemaVersion: PROFILE_SCHEMA_VERSION,
            poiCategories: DEFAULT_POI_CATEGORIES,
            fogEnabled: true
          };
        } else if (job.fog.size) {
          const fog = new Map(
            hydration.visited.map((cell) => [`${cell.x},${cell.y}`, cell])
          );
          job.fog.forEach((cell, key) => fog.set(key, cell));
          hydration.visited = Array.from(fog.values());
        }
        if (job.markers !== null) {
          hydration.localMarkers = job.markers;
        }
        if (job.settings !== null) {
          hydration.settings = {
            ...hydration.settings,
            ...job.settings
          };
        }
        if (!needsSelection && job.activeRoute !== undefined) {
          hydration.activeRoute = job.activeRoute;
        }
        window.SMMinimap?.hydrateProfileState(hydration, {
          mergeFog: !needsSelection
        });
        activeProfile = {
          sourceWorldId: job.layout.sourceWorldId,
          context: job.writeContext,
          writable: !needsSelection,
          job
        };
        if (!needsSelection) {
          captureCurrentRendererState(job);
        }
      }

      const candidates = await loadManualProfileCandidates(snapshot, job);
      isCurrent =
        latestProfileJob === job &&
        job.generation === profileGeneration &&
        window.SMMinimap?.getProfileContext?.()?.sourceWorldId ===
          job.layout.sourceWorldId;

      if (isCurrent && latestProfileJob === job) {
        profileCandidates = candidates;
        document.body.dataset.profileNeedsSplit = String(needsSelection);
        document.body.dataset.profileState = needsSelection
          ? "needs-selection"
          : "ready";
        renderProfileUi(
          snapshot,
          needsSelection ? "needs-selection" : "ready"
        );
        if (!needsSelection && job.closeDialogOnReady) {
          if (profileElements.input) {
            profileElements.input.value = "";
          }
          closeProfileDialog();
        }
        if (job.pendingTelemetry) {
          const pendingTelemetry = job.pendingTelemetry;
          job.pendingTelemetry = null;
          acceptTelemetryForActiveProfile(pendingTelemetry);
        }
      }

      if (needsSelection) {
        return;
      }

      await flushProfileJob(job);
      completeLegacyMigration(job);
    } catch (error) {
      if (latestProfileJob === job) {
        if (activeProfile?.job === job) {
          activeProfile.writable = false;
        }
        document.body.dataset.profileState = "error";
        renderProfileUi(job.snapshot, "error");
        setProfileDialogStatus(
          "Не удалось переключить профиль. Повторите попытку.",
          true
        );
      }
      console.error("ScrapMap profile activation failed", error);
    }
  }

  function requestProfileActivation(rawContext, options = {}) {
    let layout;
    try {
      layout = normalizeLayoutContext(rawContext);
    } catch (error) {
      document.body.dataset.profileState = "error";
      console.error("ScrapMap profile context was rejected", error);
      return;
    }

    const activationOptions =
      options && typeof options === "object" ? options : {};
    const hadPriorProfileContext = Boolean(latestProfileJob);
    const previousActiveProfile = activeProfile;
    const connectionObservationBaseline =
      previousActiveProfile?.job?.connectionObservationId ||
      lastConnectionObservationId;
    if (activeProfile) {
      activeProfile.writable = false;
      activeProfile = null;
    }
    window.SMMinimap?.setMarkerMode?.(false);

    if (
      latestProfileJob &&
      latestProfileJob.layout.sourceWorldId !== layout.sourceWorldId
    ) {
      profileCandidates = [];
      profileUiSnapshot = null;
      closeProfileDialog();
    }
    profileGeneration += 1;
    const job = {
      generation: profileGeneration,
      sessionId: makeSessionId(),
      layout,
      serverIdentity: activationOptions.serverIdentity || null,
      fallbackProfileId:
        boundedText(activationOptions.fallbackProfileId, 160) || null,
      fallbackProfileName:
        boundedText(activationOptions.fallbackProfileName, 80) || null,
      expectedWorldFingerprint:
        boundedText(activationOptions.expectedWorldFingerprint, 80) || null,
      connectionObservationBaseline:
        boundedText(
          activationOptions.connectionObservationId,
          96
        ) || connectionObservationBaseline || null,
      connectionObservationId:
        boundedText(activationOptions.connectionObservationId, 96) || null,
      acceptPendingInteractiveMutations:
        !previousActiveProfile && !hadPriorProfileContext,
      pendingTelemetry: null,
      lastTelemetrySequence: 0,
      validCells: new Set(layout.cells.map((cell) => `${cell.x},${cell.y}`)),
      fog: new Map(),
      markers: null,
      settings: null,
      activeRoute: undefined,
      trailBatches: [],
      legacyMigrationKey: null,
      writeContext: null,
      snapshot: null
    };
    addFogToJob(job, activationOptions.carryFog);
    latestProfileJob = job;
    const switching = Boolean(job.fallbackProfileId);
    document.body.dataset.profileState = switching ? "switching" : "loading";
    renderProfileUi(profileUiSnapshot, switching ? "switching" : "loading");
    const persistence = window.SMMinimap?.getPersistenceStatus?.();
    if (
      persistence?.sourceWorldId === layout.sourceWorldId &&
      persistence?.status !== "pending"
    ) {
      try {
        window.SMMinimap?.beginProfileHydration?.(layout.sourceWorldId);
      } catch (error) {
        console.error("ScrapMap could not enter profile hydration mode", error);
      }
    }
    enqueueStorage(() => activateProfileJob(job));
    return job;
  }

  function switchToManualProfile(fallbackProfileId, fallbackProfileName) {
    const sourceJob = latestProfileJob;
    const sourceProfile =
      sourceJob?.snapshot?.profile || profileUiSnapshot?.profile;
    if (
      !sourceJob ||
      !sourceProfile ||
      sourceProfile.scopeKind !== "fallback" ||
      document.body.dataset.profileState === "switching"
    ) {
      throw new Error("Manual profile selection is not available right now.");
    }

    if (activeProfile?.writable && activeProfile.job === sourceJob) {
      captureCurrentRendererState(sourceJob);
      enqueueStorage(() => flushProfileJob(sourceJob));
      activeProfile.writable = false;
    }
    window.SMMinimap?.setMarkerMode?.(false);

    const carryFog =
      sourceProfile.needsManualDisambiguation ||
      document.body.dataset.profileState === "error"
      ? Array.from(sourceJob.fog.values())
      : [];
    const nextJob = requestProfileActivation(sourceJob.layout, {
      serverIdentity: sourceJob.serverIdentity,
      fallbackProfileId,
      fallbackProfileName,
      expectedWorldFingerprint: sourceProfile.worldFingerprint,
      connectionObservationId: sourceJob.connectionObservationId,
      carryFog
    });
    nextJob.closeDialogOnReady = true;
    setProfileDialogStatus("Переключаем профиль…");
    return nextJob;
  }

  function selectManualProfile(fallbackProfileId) {
    const id = boundedText(fallbackProfileId, 160);
    const candidate = profileCandidates.find(
      (item) => item.fallbackProfileId === id
    );
    if (!candidate) {
      throw new TypeError("Selected manual profile is not in the current list.");
    }
    return switchToManualProfile(candidate.fallbackProfileId, null);
  }

  function createManualProfile(displayName) {
    const name = String(displayName || "").trim();
    if (
      !name ||
      [...name].length > 80 ||
      [...name].some((character) => /[\u0000-\u001f\u007f]/.test(character))
    ) {
      throw new TypeError("Введите название профиля длиной от 1 до 80 символов.");
    }
    const randomId =
      typeof window.crypto?.randomUUID === "function"
        ? window.crypto.randomUUID()
        : "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(
            /[xy]/g,
            (character) => {
              const random = Math.floor(Math.random() * 16);
              const value = character === "x" ? random : (random & 0x3) | 0x8;
              return value.toString(16);
            }
          );
    return switchToManualProfile(`manual:${randomId}`, name);
  }

  window.ScrapMapProfiles = Object.freeze({
    getState() {
      return {
        state: document.body.dataset.profileState || "idle",
        profile: profileUiSnapshot?.profile
          ? { ...profileUiSnapshot.profile }
          : null,
        candidates: profileCandidates.map((candidate) => ({ ...candidate }))
      };
    },
    open: openProfileDialog,
    close: closeProfileDialog,
    select: selectManualProfile,
    create: createManualProfile,
    refreshConnection: refreshConnectionObservation
  });

  profileElements.button?.addEventListener("click", openProfileDialog);
  profileElements.backdrop?.addEventListener("click", closeProfileDialog);
  profileElements.close?.addEventListener("click", closeProfileDialog);
  profileElements.form?.addEventListener("submit", (event) => {
    event.preventDefault();
    try {
      createManualProfile(profileElements.input?.value);
    } catch (error) {
      setProfileDialogStatus(error?.message || String(error), true);
    }
  });
  window.addEventListener(
    "keydown",
    (event) => {
      if (
        event.key === "Escape" &&
        profileElements.dialog?.classList.contains("is-open")
      ) {
        event.preventDefault();
        event.stopImmediatePropagation();
        closeProfileDialog();
      }
    },
    true
  );

  function currentMutationTarget(detail, preferLatest = false) {
    if (preferLatest && mutationMatchesJob(detail, latestProfileJob)) {
      return latestProfileJob;
    }
    const detailProfileKey = String(detail?.profileKey || "");
    if (
      activeProfile?.writable &&
      mutationMatchesJob(detail, activeProfile.job) &&
      detailProfileKey &&
      detailProfileKey === activeProfile.context?.profileKey
    ) {
      return activeProfile.job;
    }
    if (
      !activeProfile &&
      latestProfileJob?.acceptPendingInteractiveMutations &&
      !detailProfileKey &&
      mutationMatchesJob(detail, latestProfileJob)
    ) {
      return latestProfileJob;
    }
    return null;
  }

  window.addEventListener("sm-minimap:profile-context-changed", (event) => {
    requestProfileActivation(event.detail);
  });

  // Placement of the compact map is a native window concern, so it goes
  // straight through rather than into the profile store.
  window.addEventListener("sm-minimap:mini-layout", (event) => {
    const corner = Number(event.detail?.corner);
    const size = Number(event.detail?.size);
    if (!Number.isFinite(corner) || !Number.isFinite(size)) {
      return;
    }
    invoke("set_mini_overlay_layout", { size, corner }).catch((error) => {
      console.info("ScrapMap could not apply the mini-map layout", error);
    });
  });

  window.addEventListener("sm-minimap:poi-capture", () => {
    const note = document.getElementById("poiCaptureNote");
    const button = document.getElementById("poiCaptureButton");
    if (button) button.disabled = true;
    invoke("poi_capture_prepare")
      .then((result) => {
        if (note) {
          note.textContent =
            `Подготовлено объектов: ${result?.targets ?? 0}. ` +
            "Перезагрузите мир, чтобы начать съёмку.";
        }
      })
      .catch((error) => {
        if (note) note.textContent = `Не удалось подготовить съёмку: ${error}`;
      })
      .finally(() => {
        if (button) button.disabled = false;
      });
  });

  window.addEventListener("sm-minimap:fog-delta", (event) => {
    const job = currentMutationTarget(event.detail, true);
    if (!job) {
      return;
    }
    addFogToJob(job, event.detail.cells);
    if (
      activeProfile?.writable &&
      activeProfile.context === job.writeContext
    ) {
      enqueueStorage(() => flushProfileJob(job));
    }
  });

  window.addEventListener("sm-minimap:local-markers-replaced", (event) => {
    const job = currentMutationTarget(event.detail);
    if (!job) {
      return;
    }
    replaceMarkersInJob(job, event.detail.markers);
    if (
      activeProfile?.writable &&
      activeProfile.context === job.writeContext
    ) {
      enqueueStorage(() => flushProfileJob(job));
    }
  });

  window.addEventListener("sm-minimap:settings-changed", (event) => {
    const job = currentMutationTarget(event.detail);
    if (!job) {
      return;
    }
    replaceSettingsInJob(job, event.detail.settings);
    if (
      activeProfile?.writable &&
      activeProfile.context === job.writeContext
    ) {
      enqueueStorage(() => flushProfileJob(job));
    }
  });

  window.addEventListener("sm-minimap:active-route-changed", (event) => {
    const job = currentMutationTarget(event.detail);
    if (!job) {
      return;
    }
    job.activeRoute =
      event.detail.route && typeof event.detail.route === "object"
        ? { ...event.detail.route }
        : null;
    if (
      activeProfile?.writable &&
      activeProfile.context === job.writeContext
    ) {
      enqueueStorage(() => flushProfileJob(job));
    }
  });

  window.addEventListener("sm-minimap:trail-batch", (event) => {
    const job = currentMutationTarget(event.detail);
    if (!job || !event.detail.batch || typeof event.detail.batch !== "object") {
      return;
    }
    job.trailBatches.push({ ...event.detail.batch });
    if (
      activeProfile?.writable &&
      activeProfile.context === job.writeContext
    ) {
      enqueueStorage(() => flushProfileJob(job));
    }
  });

  listen("scrapmap:overlay-mode", (event) => {
    applyMode(Boolean(event.payload?.expanded));
  }).catch((error) => {
    console.error("ScrapMap overlay event subscription failed", error);
  });

  listen("scrapmap:telemetry", (event) => {
    receiveTelemetry(event.payload);
  })
    .then(async () => {
      try {
        receiveTelemetry(await invoke("diagnostic_snapshot"));
      } catch (error) {
        console.error("ScrapMap initial telemetry request failed", error);
      }
    })
    .catch((error) => {
      console.error("ScrapMap telemetry event subscription failed", error);
    });

  listen("scrapmap:diagnostic-status", (event) => {
    applyDiagnosticStatus(event.payload);
  })
    .then(async () => {
      try {
        applyDiagnosticStatus(await invoke("diagnostic_status"));
      } catch (error) {
        console.error("ScrapMap initial diagnostic status request failed", error);
      }
    })
    .catch((error) => {
      console.error("ScrapMap diagnostic status subscription failed", error);
    });

  listen("scrapmap:game-window-status", (event) => {
    applyGameWindowStatus(event.payload);
  })
    .then(async () => {
      try {
        const status = await invoke("overlay_status");
        applyGameWindowStatus(status?.game);
      } catch (error) {
        console.error("ScrapMap initial game-window status request failed", error);
      }
    })
    .catch((error) => {
      console.error("ScrapMap game-window status subscription failed", error);
    });

  const initialProfileContext = window.SMMinimap?.getProfileContext?.();
  if (initialProfileContext) {
    requestProfileActivation(initialProfileContext);
  }
  bootstrapTileAtlas();
  // The baker fills in a batch of tiles per world load, so keep picking up new
  // ones and only rebuild the atlas when something actually converted.
  window.setInterval?.(async () => {
    if ((await refreshGeneratedTiles()) > 0) {
      bootstrapTileAtlas();
    }
  }, 30000);
  refreshGameLayout();
  window.setInterval?.(refreshGameLayout, 1000);
  window.setInterval?.(refreshConnectionObservation, 1000);
})();
