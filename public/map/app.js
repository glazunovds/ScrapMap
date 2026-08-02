(function startScrapMechanicMap() {
  "use strict";

  const Core = window.SMMapCore;
  if (!Core) {
    throw new Error("map-core.js не загружен.");
  }

  const terrainPalette = {
    meadow: { base: "#60764a", detail: "#87975b", edge: "#3f5136", label: "Луга" },
    forest: { base: "#314b3a", detail: "#526d49", edge: "#23382c", label: "Лес" },
    desert: { base: "#765a39", detail: "#a07a45", edge: "#513e2c", label: "Выжженная земля" },
    water: { base: "#365c68", detail: "#4f8290", edge: "#294751", label: "Вода" },
    lake: { base: "#365c68", detail: "#4f8290", edge: "#294751", label: "Вода" },
    chemical: { base: "#5f7040", detail: "#8ca34e", edge: "#3f4d2b", label: "Химическая зона" },
    industrial: { base: "#465156", detail: "#687277", edge: "#30393c", label: "Промзона" },
    field: { base: "#7a7442", detail: "#a49a53", edge: "#554f31", label: "Поля" },
    autumn: { base: "#6c4b36", detail: "#986444", edge: "#493227", label: "Осенний лес" },
    burnt: { base: "#3f3a33", detail: "#665445", edge: "#2b2824", label: "Сгоревший лес" },
    rock: { base: "#55575a", detail: "#74777b", edge: "#383a3d", label: "Скалы" },
    unknown: { base: "#42484a", detail: "#5c6365", edge: "#303536", label: "Неизвестно" }
  };

  /**
   * Category silhouettes, drawn as canvas paths on a unit circle of the given
   * radius. A shape is recognisable at a glance where a letter is not: at the
   * sizes these render, a "W" and an "M" are the same smudge.
   */
  const poiIcons = Object.freeze({
    // Circuit chip: body with pins down each side.
    schematic(ctx, x, y, r) {
      const half = r * 0.58;
      ctx.rect(x - half, y - half, half * 2, half * 2);
      for (const offset of [-half * 0.55, half * 0.55]) {
        ctx.rect(x - half - r * 0.3, y + offset - r * 0.09, r * 0.3, r * 0.18);
        ctx.rect(x + half, y + offset - r * 0.09, r * 0.3, r * 0.18);
      }
    },
    // Exclamation mark: quest markers read as urgency.
    quest(ctx, x, y, r) {
      const w = r * 0.24;
      ctx.moveTo(x - w, y - r * 0.72);
      ctx.lineTo(x + w, y - r * 0.72);
      ctx.lineTo(x + w * 0.7, y + r * 0.16);
      ctx.lineTo(x - w * 0.7, y + r * 0.16);
      ctx.closePath();
      ctx.moveTo(x + w, y + r * 0.6);
      ctx.arc(x, y + r * 0.6, w, 0, Math.PI * 2);
    },
    // Tent.
    camp(ctx, x, y, r) {
      ctx.moveTo(x, y - r * 0.78);
      ctx.lineTo(x + r * 0.82, y + r * 0.62);
      ctx.lineTo(x - r * 0.82, y + r * 0.62);
      ctx.closePath();
    },
    // Warehouse: a wide shed with a pitched roof.
    warehouse(ctx, x, y, r) {
      ctx.moveTo(x - r * 0.85, y - r * 0.06);
      ctx.lineTo(x, y - r * 0.76);
      ctx.lineTo(x + r * 0.85, y - r * 0.06);
      ctx.lineTo(x + r * 0.85, y + r * 0.66);
      ctx.lineTo(x - r * 0.85, y + r * 0.66);
      ctx.closePath();
    },
    // Hexagonal nut, for stations and the trader.
    service(ctx, x, y, r) {
      for (let index = 0; index < 6; index += 1) {
        const angle = (Math.PI / 3) * index - Math.PI / 2;
        const px = x + Math.cos(angle) * r * 0.82;
        const py = y + Math.sin(angle) * r * 0.82;
        if (index === 0) ctx.moveTo(px, py);
        else ctx.lineTo(px, py);
      }
      ctx.closePath();
    },
    // Cave mouth: an arch standing on the ground line.
    dungeon(ctx, x, y, r) {
      ctx.moveTo(x - r * 0.76, y + r * 0.66);
      ctx.lineTo(x - r * 0.76, y);
      // Canvas y grows downward, so the dome needs the anticlockwise sweep;
      // the default direction would put the arch upside down.
      ctx.arc(x, y, r * 0.76, Math.PI, 0, true);
      ctx.lineTo(x + r * 0.76, y + r * 0.66);
      ctx.closePath();
    },
    // Four-point star for anything else worth a look.
    landmark(ctx, x, y, r) {
      const outer = r * 0.9;
      const inner = r * 0.3;
      for (let index = 0; index < 8; index += 1) {
        const angle = (Math.PI / 4) * index - Math.PI / 2;
        const length = index % 2 === 0 ? outer : inner;
        const px = x + Math.cos(angle) * length;
        const py = y + Math.sin(angle) * length;
        if (index === 0) ctx.moveTo(px, py);
        else ctx.lineTo(px, py);
      }
      ctx.closePath();
    },
    // Generator filler: a plain dot, deliberately unremarkable.
    filler(ctx, x, y, r) {
      ctx.arc(x, y, r * 0.42, 0, Math.PI * 2);
    }
  });


  const poiCategories = Object.freeze({
    schematic: {
      labelKey: "POI_CAT_SCHEMATIC",
      label: "Schematics / recipes",
      shortLabel: "СХЕМЫ",
      color: "#5ce6f2",
      fill: "#15383d"
    },
    quest: {
      labelKey: "POI_CAT_QUEST",
      label: "Quest locations",
      shortLabel: "КВЕСТ",
      color: "#c995ff",
      fill: "#30203e"
    },
    camp: {
      labelKey: "POI_CAT_CAMP",
      label: "Camp spots",
      shortLabel: "ЛАГЕРЬ",
      color: "#f0a35d",
      fill: "#3b2819"
    },
    warehouse: {
      labelKey: "POI_CAT_WAREHOUSE",
      label: "Warehouses",
      shortLabel: "СКЛАД",
      color: "#f07669",
      fill: "#3d211f"
    },
    service: {
      labelKey: "POI_CAT_STATION",
      label: "Stations and trader",
      shortLabel: "СЕРВИС",
      color: "#ffd45b",
      fill: "#3e3519"
    },
    dungeon: {
      labelKey: "POI_CAT_DUNGEON",
      label: "Dungeons",
      shortLabel: "ДАНЖ",
      color: "#6daaff",
      fill: "#1c2e45"
    },
    landmark: {
      labelKey: "POI_CAT_LANDMARK",
      label: "Other landmarks",
      shortLabel: "ПРОЧЕЕ",
      color: "#84d38c",
      fill: "#203724"
    },
    filler: {
      labelKey: "POI_CAT_FILLER",
      label: "Generator filler",
      shortLabel: "ФОН",
      color: "#8d9a94",
      fill: "#242a28"
    }
  });
  const poiCategoryOrder = Object.freeze(Object.keys(poiCategories));
  // The generator marks every random lake, roadside patch and filler spot as a
  // POI. There are over six hundred of them and they only restate what the
  // terrain already shows, so they stay off until asked for.
  const defaultPoiCategories = Object.freeze(
    poiCategoryOrder.filter((category) => category !== "filler")
  );
  const autoRevealRadius = 2;
  // Kept below the storage layer's MAX_FOG_BATCH so a full reveal splits into
  // merges it will accept.
  const FOG_DELTA_BATCH = 2000;
  const isTauriHost = Boolean(window.__TAURI__);

  const state = {
    layout: null,
    telemetry: null,
    lastLocalPlayer: null,
    visited: new Set(),
    importedMarkers: [],
    localMarkers: [],
    poiCatalog: [],
    poiSearchQuery: "",
    poiTargetId: null,
    poiTargetCellKeys: new Set(),
    poiEnabled: new Set(defaultPoiCategories),
    fogEnabled: true,
    expanded: false,
    markerMode: false,
    renderQueued: false,
    pixelRatio: 1,
    canvasWidth: 1,
    canvasHeight: 1,
    hoverCell: null,
    pointer: { x: 0, y: 0 },
    hoverPositionQueued: false,
    telemetryLive: false,
    telemetryStale: false,
    telemetryStatus: null,
    telemetryPollInFlight: false,
    telemetryPollFailures: 0,
    telemetrySignature: null,
    renderStats: {
      frames: 0,
      staticBuilds: 0,
      staticHits: 0,
      staticCellDrawCalls: 0,
      lastFrameStaticCellDrawCalls: 0,
      lastStaticBuildCellDrawCalls: 0
    },
    camera: {
      x: 0.5,
      y: 0.5,
      zoom: 43
    },
    drag: null,
    toastTimer: null,
    visitedSaveTimer: null,
    sharedVisitedPollInFlight: false,
    persistenceSuppressionDepth: 0,
    activeProfileKey: null,
    activeWorldFingerprint: null,
    activeRoute: null,
    recentTrail: null,
    profileHydrationStatus: isTauriHost ? "idle" : "browser",
    pendingVisited: new Set()
  };

  const tileAtlas = {
    manifest: null,
    entries: new Map(),
    images: new Map(),
    resolveSource: null,
    baseUrl: null,
    revision: 0
  };

  const elements = {
    body: document.body,
    canvas: document.getElementById("mapCanvas"),
    connectionLabel: document.getElementById("connectionLabel"),
    cellLabel: document.getElementById("cellLabel"),
    positionLabel: document.getElementById("positionLabel"),
    worldChip: document.getElementById("worldChip"),
    statusDot: document.getElementById("statusDot"),
    visitedMetric: document.getElementById("visitedMetric"),
    visitedDetail: document.getElementById("visitedDetail"),
    markersMetric: document.getElementById("markersMetric"),
    poiMetric: document.getElementById("poiMetric"),
    markerList: document.getElementById("markerList"),
    poiSearchInput: document.getElementById("poiSearchInput"),
    poiSearchResults: document.getElementById("poiSearchResults"),
    poiFilterList: document.getElementById("poiFilterList"),
    showAllPoiButton: document.getElementById("showAllPoiButton"),
    schematicOnlyButton: document.getElementById("schematicOnlyButton"),
    hideAllPoiButton: document.getElementById("hideAllPoiButton"),
    fogToggle: document.getElementById("fogToggle"),
    revealAllButton: document.getElementById("revealAllButton"),
    poiCaptureButton: document.getElementById("poiCaptureButton"),
    poiCaptureNote: document.getElementById("poiCaptureNote"),
    miniCornerSelect: document.getElementById("miniCornerSelect"),
    miniSizeSelect: document.getElementById("miniSizeSelect"),
    expandButton: document.getElementById("expandButton"),
    markerModeButton: document.getElementById("markerModeButton"),
    zoomInButton: document.getElementById("zoomInButton"),
    zoomOutButton: document.getElementById("zoomOutButton"),
    centerButton: document.getElementById("centerButton"),
    loadDemoButton: document.getElementById("loadDemoButton"),
    dataButton: document.getElementById("dataButton"),
    exportMarkersButton: document.getElementById("exportMarkersButton"),
    clearMarkersButton: document.getElementById("clearMarkersButton"),
    dataDrawer: document.getElementById("dataDrawer"),
    drawerBackdrop: document.getElementById("drawerBackdrop"),
    closeDrawerButton: document.getElementById("closeDrawerButton"),
    bundleInput: document.getElementById("bundleInput"),
    bundleDrop: document.getElementById("bundleDrop"),
    layoutInput: document.getElementById("layoutInput"),
    telemetryInput: document.getElementById("telemetryInput"),
    visitedInput: document.getElementById("visitedInput"),
    markersInput: document.getElementById("markersInput"),
    layoutFileStatus: document.getElementById("layoutFileStatus"),
    telemetryFileStatus: document.getElementById("telemetryFileStatus"),
    visitedFileStatus: document.getElementById("visitedFileStatus"),
    markersFileStatus: document.getElementById("markersFileStatus"),
    hoverCard: document.getElementById("hoverCard"),
    hoverCoordinates: document.getElementById("hoverCoordinates"),
    hoverTitle: document.getElementById("hoverTitle"),
    hoverDetails: document.getElementById("hoverDetails"),
    mapHint: document.getElementById("mapHint"),
    zoomLabel: document.getElementById("zoomLabel"),
    toast: document.getElementById("toast")
  };

  const hoverHighlight = document.createElement("div");
  hoverHighlight.className = "map-hover-cell";
  hoverHighlight.hidden = true;
  hoverHighlight.setAttribute("aria-hidden", "true");
  elements.canvas.insertAdjacentElement("afterend", hoverHighlight);
  elements.hoverHighlight = hoverHighlight;

  let context = elements.canvas.getContext("2d", { alpha: false });
  const staticCanvas = document.createElement("canvas");
  const staticContext = staticCanvas.getContext("2d", { alpha: false });
  const staticFrame = {
    dirty: true,
    revision: 0,
    summaryRevision: -1,
    key: null
  };

  function invalidateStaticFrame() {
    staticFrame.dirty = true;
    staticFrame.revision += 1;
  }

  function staticFrameKey(view) {
    return Core.staticFrameKey({
      revision: staticFrame.revision,
      atlasRevision: tileAtlas.revision,
      worldId: state.layout ? state.layout.worldId : "",
      expanded: state.expanded,
      width: state.canvasWidth,
      height: state.canvasHeight,
      pixelRatio: state.pixelRatio,
      camera: view.camera,
      scale: view.scale
    });
  }

  function hashString(value) {
    let hash = 2166136261;
    const string = String(value);
    for (let index = 0; index < string.length; index += 1) {
      hash ^= string.charCodeAt(index);
      hash = Math.imul(hash, 16777619);
    }
    return hash >>> 0;
  }

  function pseudoRandom(seed, index) {
    let value = (seed + Math.imul(index + 1, 0x6d2b79f5)) >>> 0;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  }

  function getPlayerGridPosition(player) {
    if (!state.layout || !state.telemetry) {
      return { x: 0.5, y: 0.5 };
    }
    const size = state.layout.cellSize;
    const position = player || state.lastLocalPlayer || state.telemetry.player;
    return {
      x: position.x / size,
      y: position.y / size
    };
  }

  function getScale() {
    if (state.expanded) {
      return Core.clamp(
        state.camera.zoom,
        Core.minimumExpandedZoom({
          fogEnabled: state.fogEnabled,
          visitedKeys: state.visited,
          viewportWidth: state.canvasWidth,
          viewportHeight: state.canvasHeight,
          margin: 5,
          absoluteMinimum: 14,
          maximum: 116
        }),
        116
      );
    }
    const shortestSide = Math.max(320, Math.min(state.canvasWidth, state.canvasHeight));
    return Core.clamp(shortestSide / 10.5, 28, 70);
  }

  function getCamera() {
    if (state.expanded) {
      return state.camera;
    }
    const player = getPlayerGridPosition();
    return { x: player.x, y: player.y, zoom: getScale() };
  }

  function gridToScreen(gridX, gridY, view) {
    const camera = view ? view.camera : getCamera();
    const scale = view ? view.scale : getScale();
    return {
      x: (gridX - camera.x) * scale + state.canvasWidth / 2,
      y: (camera.y - gridY) * scale + state.canvasHeight / 2
    };
  }

  function screenToGrid(screenX, screenY) {
    const camera = getCamera();
    const scale = getScale();
    return {
      x: camera.x + (screenX - state.canvasWidth / 2) / scale,
      y: camera.y - (screenY - state.canvasHeight / 2) / scale
    };
  }

  function screenToCell(screenX, screenY) {
    const grid = screenToGrid(screenX, screenY);
    return { x: Math.floor(grid.x), y: Math.floor(grid.y) };
  }

  function isVisited(cell) {
    return state.visited.has(cell.key);
  }

  function terrainStyle(terrain) {
    return terrainPalette[terrain] || terrainPalette.unknown;
  }

  function atlasSource(entry) {
    if (typeof entry?.source === "string" && entry.source) {
      return entry.source;
    }
    if (typeof tileAtlas.resolveSource === "function") {
      try {
        const source = tileAtlas.resolveSource(entry);
        if (typeof source === "string" && source) {
          return source;
        }
        if (source && typeof source.then === "function") {
          return source;
        }
        return null;
      } catch (_error) {
        return null;
      }
    }
    if (tileAtlas.baseUrl && typeof entry?.relativePath === "string") {
      try {
        const baseUrl = tileAtlas.baseUrl.endsWith("/")
          ? tileAtlas.baseUrl
          : `${tileAtlas.baseUrl}/`;
        return new URL(entry.relativePath, baseUrl).toString();
      } catch (_error) {
        return null;
      }
    }
    return null;
  }

  function rectifiedAtlasPreview(image) {
    if (typeof document?.createElement !== "function") {
      return image;
    }

    const width = image.naturalWidth || image.width;
    const height = image.naturalHeight || image.height;
    if (!width || !height) {
      return image;
    }

    try {
      const source = document.createElement("canvas");
      source.width = width;
      source.height = height;
      const sourceContext = source.getContext("2d", { willReadFrequently: true });
      if (!sourceContext) return image;
      sourceContext.drawImage(image, 0, 0);

      const frame = sourceContext.getImageData(0, 0, width, height);
      const pixels = frame.data;
      const corners = [0, width - 1, (height - 1) * width, height * width - 1];
      const background = corners.reduce(
        (sum, index) => {
          const offset = index * 4;
          sum[0] += pixels[offset];
          sum[1] += pixels[offset + 1];
          sum[2] += pixels[offset + 2];
          return sum;
        },
        [0, 0, 0]
      ).map((channel) => channel / corners.length);

      const rowCounts = new Uint16Array(height);
      const columnCounts = new Uint16Array(width);
      for (let y = 0; y < height; y += 1) {
        for (let x = 0; x < width; x += 1) {
          const offset = (y * width + x) * 4;
          const difference = Math.max(
            Math.abs(pixels[offset] - background[0]),
            Math.abs(pixels[offset + 1] - background[1]),
            Math.abs(pixels[offset + 2] - background[2])
          );
          if (pixels[offset + 3] > 8 && difference > 14) {
            rowCounts[y] += 1;
            columnCounts[x] += 1;
          } else {
            pixels[offset + 3] = 0;
          }
        }
      }
      sourceContext.putImageData(frame, 0, 0);

      const minimumRowPixels = Math.max(2, Math.floor(width * 0.01));
      const minimumColumnPixels = Math.max(2, Math.floor(height * 0.01));
      let minY = 0;
      let maxY = height - 1;
      let minX = 0;
      let maxX = width - 1;
      while (minY < maxY && rowCounts[minY] < minimumRowPixels) minY += 1;
      while (maxY > minY && rowCounts[maxY] < minimumRowPixels) maxY -= 1;
      while (minX < maxX && columnCounts[minX] < minimumColumnPixels) minX += 1;
      while (maxX > minX && columnCounts[maxX] < minimumColumnPixels) maxX -= 1;

      const centerX = (minX + maxX) / 2;
      const centerY = (minY + maxY) / 2;
      const topPoint = { x: centerX, y: minY };
      const rightPoint = { x: maxX, y: centerY };
      const leftPoint = { x: minX, y: centerY };
      const first = {
        x: rightPoint.x - topPoint.x,
        y: rightPoint.y - topPoint.y
      };
      const second = {
        x: leftPoint.x - topPoint.x,
        y: leftPoint.y - topPoint.y
      };
      const determinant = first.x * second.y - first.y * second.x;
      if (Math.abs(determinant) < 1) return image;

      const size = 256;
      const result = document.createElement("canvas");
      result.width = size;
      result.height = size;
      const resultContext = result.getContext("2d");
      if (!resultContext) return image;
      const a = (size * second.y) / determinant;
      const c = (-size * second.x) / determinant;
      const b = (-size * first.y) / determinant;
      const d = (size * first.x) / determinant;
      resultContext.setTransform(
        a,
        b,
        c,
        d,
        -a * topPoint.x - c * topPoint.y,
        -b * topPoint.x - d * topPoint.y
      );
      resultContext.drawImage(source, 0, 0);
      return result;
    } catch (_error) {
      return image;
    }
  }

  function atlasImage(entry) {
    const key = entry.topDownRelativePath || entry.relativePath;
    const cached = tileAtlas.images.get(key);
    if (cached) {
      return cached.status === "ready" ? cached.image : null;
    }

    if (typeof window.Image !== "function") {
      return null;
    }

    const image = new window.Image();
    const record = { image, status: "loading" };
    tileAtlas.images.set(key, record);
    image.onload = () => {
      record.image = entry.topDownRelativePath ? image : rectifiedAtlasPreview(image);
      record.status = "ready";
      tileAtlas.revision += 1;
      invalidateStaticFrame();
      scheduleRender();
      // The search list may have been drawn before this arrived.
      if (elements.poiSearchResults && !elements.poiSearchResults.hidden) {
        renderPoiSearch();
      }
    };
    image.onerror = () => {
      record.status = "failed";
      tileAtlas.revision += 1;
      invalidateStaticFrame();
      scheduleRender();
    };

    const fail = () => {
      record.status = "failed";
      tileAtlas.revision += 1;
      invalidateStaticFrame();
      scheduleRender();
    };
    const start = (source) => {
      if (typeof source !== "string" || !source) {
        fail();
        return;
      }
      image.src = source;
    };
    try {
      const source = atlasSource(entry);
      if (source && typeof source.then === "function") {
        Promise.resolve(source).then(start, fail);
      } else {
        start(source);
      }
    } catch (_error) {
      fail();
    }
    return null;
  }

  function drawAtlasTile(cell, left, top, scale) {
    if (!tileAtlas.entries.size) {
      return false;
    }
    const tileUuid = String(cell.tileKey || cell.uuid || "").toLowerCase();
    const entry = tileAtlas.entries.get(tileUuid);
    if (!entry?.topDownRelativePath) {
      return false;
    }
    const image = entry ? atlasImage(entry) : null;
    if (!image) {
      return false;
    }

    const width = image.naturalWidth || image.width;
    const height = image.naturalHeight || image.height;
    if (!width || !height) {
      return false;
    }

    // A tile image covers the whole tile, which may span several cells. Take
    // only this cell's slice, or a 4x4 tile would be drawn complete in each of
    // its sixteen cells.
    // Prefer the atlas entry: the persisted layout drops tileSize, so relying
    // on the cell alone would break after a restart that reloads from storage.
    const size = Math.max(
      1,
      Math.round(Number(entry.tileSize) || Number(cell.tileSize) || 1)
    );
    const sliceWidth = width / size;
    const sliceHeight = height / size;
    const offsetX = Math.min(size - 1, Math.max(0, Math.round(Number(cell.xOffset) || 0)));
    const offsetY = Math.min(size - 1, Math.max(0, Math.round(Number(cell.yOffset) || 0)));
    // Image rows run north to south while tile offsets count northward.
    const sourceX = offsetX * sliceWidth;
    const sourceY = (size - 1 - offsetY) * sliceHeight;

    context.save();
    context.beginPath();
    context.rect(left, top, scale, scale);
    context.clip();
    context.globalAlpha = 0.94;
    context.translate(left + scale / 2, top + scale / 2);
    const rotation = ((4 - Number(cell.rotation || 0)) % 4) * (Math.PI / 2);
    context.rotate(rotation);
    context.drawImage(
      image,
      sourceX,
      sourceY,
      sliceWidth,
      sliceHeight,
      -scale / 2,
      -scale / 2,
      scale,
      scale
    );
    context.restore();
    return true;
  }

  function setTileAtlas(manifest, options = {}) {
    const entries = Array.isArray(manifest?.entries) ? manifest.entries : [];
    const indexedEntries = new Map();
    entries.forEach((entry) => {
      const tileUuid = String(entry?.tileUuid || "").toLowerCase();
      const relativePath = String(entry?.relativePath || "");
      if (tileUuid && relativePath && !indexedEntries.has(tileUuid)) {
        indexedEntries.set(tileUuid, {
          ...entry,
          tileUuid,
          relativePath
        });
      }
    });
    tileAtlas.manifest = manifest && typeof manifest === "object" ? manifest : null;
    tileAtlas.entries = indexedEntries;
    tileAtlas.images.clear();
    tileAtlas.resolveSource =
      typeof options?.resolveSource === "function" ? options.resolveSource : null;
    tileAtlas.baseUrl = typeof options?.baseUrl === "string" ? options.baseUrl : null;
    tileAtlas.revision += 1;
    invalidateStaticFrame();
    scheduleRender();
    return {
      fileCount: entries.length,
      uniqueTileIds: indexedEntries.size,
      contentFingerprint: String(manifest?.contentFingerprint || "") || null
    };
  }

  function drawBackground() {
    const gradient = context.createRadialGradient(
      state.canvasWidth * 0.42,
      state.canvasHeight * 0.45,
      0,
      state.canvasWidth * 0.42,
      state.canvasHeight * 0.45,
      Math.max(state.canvasWidth, state.canvasHeight) * 0.76
    );
    gradient.addColorStop(0, "#182022");
    gradient.addColorStop(0.64, "#101617");
    gradient.addColorStop(1, "#0a0e0f");
    context.fillStyle = gradient;
    context.fillRect(0, 0, state.canvasWidth, state.canvasHeight);

    context.save();
    context.globalAlpha = 0.06;
    context.strokeStyle = "#d7d9cf";
    context.lineWidth = 1;
    const step = 24;
    for (let x = -state.canvasHeight; x < state.canvasWidth + state.canvasHeight; x += step) {
      context.beginPath();
      context.moveTo(x, 0);
      context.lineTo(x + state.canvasHeight, state.canvasHeight);
      context.stroke();
    }
    context.restore();
  }

  function drawTerrainTexture(cell, left, top, scale, style) {
    const seed = hashString(`${cell.tileKey}:${cell.x}:${cell.y}`);
    context.save();
    context.beginPath();
    context.rect(left, top, scale, scale);
    context.clip();

    context.globalAlpha = 0.2;
    context.strokeStyle = style.detail;
    context.lineWidth = Math.max(0.7, scale * 0.014);
    const contourCount = scale > 38 ? 4 : 2;
    for (let index = 0; index < contourCount; index += 1) {
      const y = top + pseudoRandom(seed, index) * scale;
      const amplitude = scale * (0.04 + pseudoRandom(seed, index + 10) * 0.07);
      context.beginPath();
      context.moveTo(left - 3, y);
      context.bezierCurveTo(
        left + scale * 0.27,
        y - amplitude,
        left + scale * 0.62,
        y + amplitude,
        left + scale + 3,
        y - amplitude * 0.25
      );
      context.stroke();
    }

    const fleckCount = scale > 45 ? 8 : 4;
    context.fillStyle = style.detail;
    for (let index = 0; index < fleckCount; index += 1) {
      const x = left + pseudoRandom(seed, index + 20) * scale;
      const y = top + pseudoRandom(seed, index + 40) * scale;
      const radius = Math.max(0.7, scale * (0.012 + pseudoRandom(seed, index + 60) * 0.02));
      context.beginPath();
      context.arc(x, y, radius, 0, Math.PI * 2);
      context.fill();
    }
    context.restore();
  }

  function roadEndpoint(direction, centerX, centerY, half) {
    if (direction === "n") return { x: centerX, y: centerY - half - 1 };
    if (direction === "e") return { x: centerX + half + 1, y: centerY };
    if (direction === "s") return { x: centerX, y: centerY + half + 1 };
    return { x: centerX - half - 1, y: centerY };
  }

  function drawRoads(cell, left, top, scale) {
    const roads = Core.rotateRoads(cell.roads, cell.rotation);
    if (!roads.length) {
      return;
    }

    const centerX = left + scale / 2;
    const centerY = top + scale / 2;
    const half = scale / 2;
    const roadWidth = Math.max(3.5, scale * 0.16);

    context.save();
    context.lineCap = "butt";
    context.strokeStyle = "#252b2c";
    context.lineWidth = roadWidth + Math.max(2, scale * 0.055);
    roads.forEach((direction) => {
      const endpoint = roadEndpoint(direction, centerX, centerY, half);
      context.beginPath();
      context.moveTo(centerX, centerY);
      context.lineTo(endpoint.x, endpoint.y);
      context.stroke();
    });

    context.strokeStyle = "#7f8178";
    context.lineWidth = roadWidth;
    roads.forEach((direction) => {
      const endpoint = roadEndpoint(direction, centerX, centerY, half);
      context.beginPath();
      context.moveTo(centerX, centerY);
      context.lineTo(endpoint.x, endpoint.y);
      context.stroke();
    });

    if (scale > 31) {
      context.strokeStyle = "#d3bd72";
      context.lineWidth = Math.max(0.7, scale * 0.018);
      context.setLineDash([scale * 0.09, scale * 0.08]);
      roads.forEach((direction) => {
        const endpoint = roadEndpoint(direction, centerX, centerY, half);
        context.beginPath();
        context.moveTo(centerX, centerY);
        context.lineTo(endpoint.x, endpoint.y);
        context.stroke();
      });
      context.setLineDash([]);
    }
    context.restore();
  }

  function drawPoi(cell, left, top, scale) {
    if (!cell.poi || !state.poiEnabled.has(cell.poi.category || "landmark")) {
      return;
    }

    const category = cell.poi.category || "landmark";
    const poiStyle = poiCategories[category] || poiCategories.landmark;
    const radius = Core.clamp(scale * 0.16, 4.5, 13);
    const x = left + scale / 2;
    const y = top + scale / 2;
    const drawIcon = poiIcons[category] || poiIcons.landmark;

    context.save();
    // The badge keeps the icon legible over any terrain; the silhouette on top
    // is what actually identifies the category.
    context.shadowColor = "rgba(0, 0, 0, 0.72)";
    context.shadowBlur = 7;
    context.fillStyle = poiStyle.fill;
    context.strokeStyle = poiStyle.color;
    context.lineWidth = Math.max(1.2, radius * 0.18);
    context.beginPath();
    context.arc(x, y, radius, 0, Math.PI * 2);
    context.fill();
    context.stroke();
    context.shadowBlur = 0;

    context.fillStyle = poiStyle.color;
    context.beginPath();
    drawIcon(context, x, y, radius * 0.72);
    context.fill();
    if (state.poiTargetCellKeys.has(cell.key)) {
      context.strokeStyle = "#fff1b8";
      context.lineWidth = Math.max(1.4, radius * 0.16);
      context.beginPath();
      context.arc(x, y, radius + Math.max(3, scale * 0.08), 0, Math.PI * 2);
      context.stroke();
    }
    context.restore();
  }

  function drawUnknownCell(cell, left, top, scale) {
    const seed = hashString(`unknown:${cell.x}:${cell.y}`);
    context.save();
    context.fillStyle = "#151b1d";
    context.fillRect(left, top, scale, scale);

    context.beginPath();
    context.rect(left, top, scale, scale);
    context.clip();
    for (let index = 0; index < 2; index += 1) {
      const cloudX = left + pseudoRandom(seed, index + 2) * scale;
      const cloudY = top + pseudoRandom(seed, index + 7) * scale;
      const cloudRadius = scale * (0.34 + pseudoRandom(seed, index + 12) * 0.3);
      context.fillStyle = index === 0
        ? "rgba(111, 126, 127, 0.08)"
        : "rgba(3, 7, 8, 0.14)";
      context.beginPath();
      context.arc(cloudX, cloudY, cloudRadius, 0, Math.PI * 2);
      context.fill();
    }
    context.restore();
  }

  function drawFogFrontier(cell, left, top, scale) {
    const edges = [
      { visited: state.visited.has(Core.cellKey(cell.x, cell.y + 1)), x0: 0, y0: 0, x1: 0, y1: 1, x: 0, y: 0, w: 1, h: 0.36 },
      { visited: state.visited.has(Core.cellKey(cell.x + 1, cell.y)), x0: 1, y0: 0, x1: 0, y1: 0, x: 0.64, y: 0, w: 0.36, h: 1 },
      { visited: state.visited.has(Core.cellKey(cell.x, cell.y - 1)), x0: 0, y0: 1, x1: 0, y1: 0, x: 0, y: 0.64, w: 1, h: 0.36 },
      { visited: state.visited.has(Core.cellKey(cell.x - 1, cell.y)), x0: 0, y0: 0, x1: 1, y1: 0, x: 0, y: 0, w: 0.36, h: 1 }
    ];
    context.save();
    edges.forEach((edge) => {
      if (!edge.visited) return;
      const gradient = context.createLinearGradient(
        left + edge.x0 * scale,
        top + edge.y0 * scale,
        left + edge.x1 * scale,
        top + edge.y1 * scale
      );
      gradient.addColorStop(0, "rgba(245, 191, 86, 0.2)");
      gradient.addColorStop(0.18, "rgba(120, 134, 125, 0.12)");
      gradient.addColorStop(1, "rgba(21, 27, 29, 0)");
      context.fillStyle = gradient;
      context.fillRect(
        left + edge.x * scale,
        top + edge.y * scale,
        edge.w * scale,
        edge.h * scale
      );
    });
    context.restore();
  }

  function drawCell(cell, view) {
    const { scale } = view;
    const topLeft = gridToScreen(cell.x, cell.y + 1, view);
    const left = Math.floor(topLeft.x);
    const top = Math.floor(topLeft.y);
    if (left > state.canvasWidth || top > state.canvasHeight || left + scale < 0 || top + scale < 0) {
      return;
    }

    const visited = isVisited(cell);
    if (state.fogEnabled && !visited) {
      drawUnknownCell(cell, left, top, scale);
      drawFogFrontier(cell, left, top, scale);
      context.strokeStyle = "rgba(185, 196, 192, 0.055)";
      context.lineWidth = 1;
      context.strokeRect(left + 0.5, top + 0.5, Math.max(0, scale - 1), Math.max(0, scale - 1));
      return;
    }

    const style = terrainStyle(cell.terrain);
    context.fillStyle = style.base;
    context.fillRect(left, top, Math.ceil(scale), Math.ceil(scale));
    const hasAtlasTile = drawAtlasTile(cell, left, top, scale);
    if (!hasAtlasTile) {
      drawTerrainTexture(cell, left, top, scale, style);
      drawRoads(cell, left, top, scale);
    }
    drawPoi(cell, left, top, scale);

    context.strokeStyle = scale > 23 ? "rgba(230, 235, 226, 0.12)" : "rgba(230, 235, 226, 0.075)";
    context.lineWidth = 1;
    context.strokeRect(left + 0.5, top + 0.5, Math.max(0, scale - 1), Math.max(0, scale - 1));
  }

  function drawPoiOverlays(view) {
    if (!state.layout || !tileAtlas.entries.size) return;
    const drawnGroups = new Set();
    state.layout.cells.forEach((cell) => {
      if (cell.xOffset !== 0 || cell.yOffset !== 0) return;
      const entry = tileAtlas.entries.get(String(cell.uuid || cell.tileKey).toLowerCase());
      if (!entry?.poiOverlayRelativePath) return;
      const groupKey = cell.groupId > 0 ? `group:${cell.groupId}` : `cell:${cell.key}`;
      if (drawnGroups.has(groupKey)) return;
      drawnGroups.add(groupKey);
      if (state.fogEnabled && !isVisited(cell)) return;

      const size = Math.max(1, Number(entry.poiOverlayTileSize || cell.tileSize || 1));
      const image = atlasImage({
        relativePath: entry.poiOverlayRelativePath,
        topDownRelativePath: entry.poiOverlayRelativePath
      });
      if (!image) return;
      const offsetX = Number(entry.poiOverlayOffsetX || 0);
      const offsetY = Number(entry.poiOverlayOffsetY || 0);
      const topLeft = gridToScreen(
        cell.x + offsetX,
        cell.y + offsetY + size,
        view
      );
      const drawSize = size * view.scale;
      context.save();
      context.beginPath();
      context.rect(topLeft.x, topLeft.y, drawSize, drawSize);
      context.clip();
      context.translate(topLeft.x + drawSize / 2, topLeft.y + drawSize / 2);
      context.rotate(((4 - Number(cell.rotation || 0)) % 4) * (Math.PI / 2));
      context.globalAlpha = 0.98;
      context.drawImage(image, -drawSize / 2, -drawSize / 2, drawSize, drawSize);
      context.restore();
    });
  }

  function drawWorldBounds(view) {
    if (!state.layout || !state.expanded) {
      return;
    }
    const bounds = state.layout.bounds;
    const topLeft = gridToScreen(bounds.minX, bounds.maxY + 1, view);
    const bottomRight = gridToScreen(bounds.maxX + 1, bounds.minY, view);
    context.save();
    context.strokeStyle = "rgba(245, 185, 66, 0.28)";
    context.lineWidth = 1;
    context.setLineDash([6, 6]);
    context.strokeRect(
      topLeft.x - 2,
      topLeft.y - 2,
      bottomRight.x - topLeft.x + 4,
      bottomRight.y - topLeft.y + 4
    );
    context.restore();
  }

  function allMarkers() {
    const markers = new Map();
    state.importedMarkers.forEach((marker) => markers.set(marker.id, marker));
    state.localMarkers.forEach((marker) => markers.set(marker.id, marker));
    return Array.from(markers.values());
  }

  function drawMarkers(view) {
    const { scale } = view;
    allMarkers().forEach((marker) => {
      const point = gridToScreen(marker.cellX + 0.5, marker.cellY + 0.5, view);
      if (point.x < -30 || point.x > state.canvasWidth + 30 || point.y < -30 || point.y > state.canvasHeight + 30) {
        return;
      }
      const size = Core.clamp(scale * 0.23, 7, 16);
      context.save();
      context.shadowColor = "rgba(0, 0, 0, 0.8)";
      context.shadowBlur = 8;
      context.strokeStyle = marker.local ? "#ff7464" : "#e6a096";
      context.lineWidth = Core.clamp(size * 0.22, 2, 3.5);
      context.lineCap = "round";
      context.beginPath();
      context.moveTo(point.x - size, point.y - size);
      context.lineTo(point.x + size, point.y + size);
      context.moveTo(point.x + size, point.y - size);
      context.lineTo(point.x - size, point.y + size);
      context.stroke();
      context.shadowBlur = 0;

      if (state.expanded && scale > 43) {
        context.fillStyle = "rgba(12, 15, 16, 0.8)";
        const labelWidth = Math.min(160, context.measureText(marker.label).width + 15);
        context.fillRect(point.x + size + 6, point.y - 10, labelWidth, 19);
        context.fillStyle = "#efc5bf";
        context.font = "9px Segoe UI, sans-serif";
        context.textAlign = "left";
        context.textBaseline = "middle";
        context.fillText(marker.label, point.x + size + 11, point.y, 145);
      }
      context.restore();
    });
  }

  function telemetryPlayersInCurrentWorld() {
    if (!state.telemetry || !state.telemetry.player) {
      return [];
    }
    const localPlayer = state.telemetry.player;
    const players = state.telemetry.players && state.telemetry.players.length
      ? state.telemetry.players
      : [localPlayer];
    return players.filter((player) =>
      player.active !== false &&
      player.hasCharacter !== false &&
      player.sameWorld !== false &&
      (
        player.worldId == null ||
        localPlayer.worldId == null ||
        player.worldId === localPlayer.worldId
      )
    );
  }

  function drawPlayerMarker(player, view, local) {
    const { scale } = view;
    const gridPosition = getPlayerGridPosition(player);
    const point = gridToScreen(gridPosition.x, gridPosition.y, view);
    if (
      point.x < -80 ||
      point.x > state.canvasWidth + 80 ||
      point.y < -80 ||
      point.y > state.canvasHeight + 80
    ) {
      return null;
    }

    const radius = Core.clamp(scale * (local ? 0.19 : 0.16), 6, local ? 14 : 12);
    const headingRadians = (player.heading * Math.PI) / 180;
    const remoteHue = 174 + (hashString(player.id || player.name) % 58);
    const fillColor = local ? "#ffcb51" : `hsl(${remoteHue} 78% 61%)`;
    const strokeColor = local ? "#fff1b8" : `hsl(${remoteHue} 88% 87%)`;

    context.save();
    if (local) {
      context.strokeStyle = "rgba(255, 209, 92, 0.22)";
      context.lineWidth = 1;
      context.beginPath();
      context.arc(point.x, point.y, Math.max(scale * 1.5, 44), 0, Math.PI * 2);
      context.stroke();
    }

    context.translate(point.x, point.y);
    context.rotate(headingRadians);
    context.shadowColor = "rgba(0, 0, 0, 0.75)";
    context.shadowBlur = 9;
    context.fillStyle = fillColor;
    context.strokeStyle = strokeColor;
    context.lineWidth = local ? 1.4 : 1.2;
    context.beginPath();
    context.moveTo(0, -radius * 1.35);
    context.lineTo(radius * 0.82, radius);
    context.lineTo(0, radius * 0.65);
    context.lineTo(-radius * 0.82, radius);
    context.closePath();
    context.fill();
    context.stroke();
    context.restore();
    return { player, local, point, radius, strokeColor };
  }

  function playerLabelBounds() {
    let right = state.canvasWidth - 8;
    if (state.expanded) {
      const panel = document.querySelector(".side-panel");
      if (panel) {
        const canvasBounds = elements.canvas.getBoundingClientRect();
        const panelBounds = panel.getBoundingClientRect();
        if (panelBounds.left > canvasBounds.left && panelBounds.left < canvasBounds.right) {
          right = Math.min(right, panelBounds.left - canvasBounds.left - 8);
        }
      }
    }
    return { left: 8, top: 8, right: Math.max(168, right), bottom: state.canvasHeight - 8 };
  }

  function drawPlayerLabel(marker, occupied) {
    const { player, local, point, radius, strokeColor } = marker;
    const label = Core.playerDisplayName(player);
    const fontSize = state.expanded ? 11 : 10;
    const height = state.expanded ? 24 : 22;
    const maximumWidth = state.expanded ? 190 : 156;
    context.save();
    context.font = `650 ${fontSize}px Segoe UI, sans-serif`;
    const width = Math.min(maximumWidth, Math.max(42, context.measureText(label).width + 18));
    const gap = radius + 8;
    const candidates = [
      { x: point.x + gap, y: point.y - height / 2, width, height },
      { x: point.x - gap - width, y: point.y - height / 2, width, height },
      { x: point.x - width / 2, y: point.y - gap - height, width, height },
      { x: point.x - width / 2, y: point.y + gap, width, height },
      { x: point.x + gap, y: point.y - gap - height, width, height },
      { x: point.x - gap - width, y: point.y - gap - height, width, height },
      { x: point.x + gap, y: point.y + gap, width, height },
      { x: point.x - gap - width, y: point.y + gap, width, height }
    ];
    const rect = Core.chooseLabelRect(candidates, occupied, playerLabelBounds());
    if (!rect) {
      context.restore();
      return;
    }
    occupied.push(rect);
    context.shadowColor = "rgba(0, 0, 0, 0.72)";
    context.shadowBlur = 6;
    context.fillStyle = local ? "rgba(45, 36, 13, 0.92)" : "rgba(8, 13, 15, 0.9)";
    context.fillRect(rect.x, rect.y, rect.width, rect.height);
    context.shadowBlur = 0;
    context.strokeStyle = local ? "rgba(255, 203, 81, 0.58)" : "rgba(130, 222, 235, 0.35)";
    context.lineWidth = 1;
    context.strokeRect(rect.x + 0.5, rect.y + 0.5, rect.width - 1, rect.height - 1);
    context.fillStyle = strokeColor;
    context.textAlign = "left";
    context.textBaseline = "middle";
    context.fillText(label, rect.x + 9, rect.y + rect.height / 2, rect.width - 18);
    context.restore();
  }

  function drawPlayers(view) {
    if (!state.telemetry || !state.layout) {
      return;
    }
    const players = telemetryPlayersInCurrentWorld();
    const localPlayer =
      players.find((player) => player.local) ||
      state.lastLocalPlayer;
    const orderedPlayers = players
      .filter((player) => !player.local)
      .map((player) => ({ player, local: false }));
    if (localPlayer) orderedPlayers.push({ player: localPlayer, local: true });

    const markers = orderedPlayers
      .map((entry) => drawPlayerMarker(entry.player, view, entry.local))
      .filter(Boolean);
    const markerBlockers = markers.map((marker) => ({
      x: marker.point.x - marker.radius - 4,
      y: marker.point.y - marker.radius - 4,
      width: marker.radius * 2 + 8,
      height: marker.radius * 2 + 8
    }));
    const occupied = markerBlockers.slice();
    markers.forEach((marker) => drawPlayerLabel(marker, occupied));
  }

  function drawEmptyState() {
    context.save();
    context.fillStyle = "#dce1dc";
    context.textAlign = "center";
    context.font = "600 16px Segoe UI, sans-serif";
    context.fillText("Нет данных layout", state.canvasWidth / 2, state.canvasHeight / 2 - 8);
    context.fillStyle = "#7f8988";
    context.font = "11px Segoe UI, sans-serif";
    context.fillText("Откройте «Данные» и выберите JSON", state.canvasWidth / 2, state.canvasHeight / 2 + 17);
    context.restore();
  }

  function rebuildStaticFrame(view, key) {
    const physicalWidth = Math.max(1, Math.round(state.canvasWidth * state.pixelRatio));
    const physicalHeight = Math.max(1, Math.round(state.canvasHeight * state.pixelRatio));
    if (staticCanvas.width !== physicalWidth || staticCanvas.height !== physicalHeight) {
      staticCanvas.width = physicalWidth;
      staticCanvas.height = physicalHeight;
    }

    const primaryContext = context;
    let cellDrawCalls = 0;
    context = staticContext;
    try {
      context.setTransform(state.pixelRatio, 0, 0, state.pixelRatio, 0, 0);
      context.clearRect(0, 0, state.canvasWidth, state.canvasHeight);
      drawBackground();
      Core.forEachVisibleCell(
        state.layout,
        view.camera,
        state.canvasWidth,
        state.canvasHeight,
        view.scale,
        (cell) => {
          cellDrawCalls += 1;
          drawCell(cell, view);
        },
        1
      );
      drawPoiOverlays(view);
      drawWorldBounds(view);
      drawMarkers(view);
    } finally {
      context = primaryContext;
    }

    staticFrame.key = key;
    staticFrame.dirty = false;
    state.renderStats.staticBuilds += 1;
    state.renderStats.staticCellDrawCalls += cellDrawCalls;
    state.renderStats.lastFrameStaticCellDrawCalls = cellDrawCalls;
    state.renderStats.lastStaticBuildCellDrawCalls = cellDrawCalls;
  }

  function drawStaticFrame(view) {
    const key = staticFrameKey(view);
    if (staticFrame.dirty || staticFrame.key !== key) {
      rebuildStaticFrame(view, key);
    } else {
      state.renderStats.staticHits += 1;
      state.renderStats.lastFrameStaticCellDrawCalls = 0;
    }
    context.drawImage(
      staticCanvas,
      0,
      0,
      staticCanvas.width,
      staticCanvas.height,
      0,
      0,
      state.canvasWidth,
      state.canvasHeight
    );
  }

  function render() {
    state.renderQueued = false;
    state.renderStats.frames += 1;
    context.setTransform(state.pixelRatio, 0, 0, state.pixelRatio, 0, 0);
    context.clearRect(0, 0, state.canvasWidth, state.canvasHeight);

    if (!state.layout) {
      drawBackground();
      drawEmptyState();
      return;
    }

    const view = {
      camera: getCamera(),
      scale: getScale()
    };
    drawStaticFrame(view);
    drawPlayers(view);
    elements.canvas.dataset.lastStaticCellDrawCalls =
      String(state.renderStats.lastFrameStaticCellDrawCalls);
    elements.canvas.dataset.staticCacheBuilds =
      String(state.renderStats.staticBuilds);
    elements.canvas.dataset.staticCacheHits =
      String(state.renderStats.staticHits);
    elements.canvas.dataset.lastStaticBuildCellDrawCalls =
      String(state.renderStats.lastStaticBuildCellDrawCalls);
    positionHoverHighlight(view);
    elements.zoomLabel.textContent = `МАСШТАБ ${Math.round((view.scale / 43) * 100)}%`;
  }

  function scheduleRender() {
    if (!state.renderQueued) {
      state.renderQueued = true;
      window.requestAnimationFrame(render);
    }
  }

  function resizeCanvas() {
    const bounds = elements.canvas.getBoundingClientRect();
    const ratio = Math.min(2, Math.max(1, window.devicePixelRatio || 1));
    const width = Math.max(1, Math.round(bounds.width));
    const height = Math.max(1, Math.round(bounds.height));
    if (
      elements.canvas.width !== Math.round(width * ratio) ||
      elements.canvas.height !== Math.round(height * ratio)
    ) {
      elements.canvas.width = Math.round(width * ratio);
      elements.canvas.height = Math.round(height * ratio);
      state.pixelRatio = ratio;
      state.canvasWidth = width;
      state.canvasHeight = height;
      invalidateStaticFrame();
      scheduleRender();
    }
  }

  function withPersistenceSuppressed(callback) {
    state.persistenceSuppressionDepth += 1;
    try {
      return callback();
    } finally {
      state.persistenceSuppressionDepth -= 1;
    }
  }

  function persistenceIsSuppressed() {
    return state.persistenceSuppressionDepth > 0;
  }

  function dispatchPersistenceEvent(type, detail) {
    if (!isTauriHost || persistenceIsSuppressed()) {
      return;
    }
    window.dispatchEvent(new CustomEvent(type, { detail }));
  }

  function layoutProfileContext() {
    if (!state.layout) {
      return null;
    }
    return {
      schemaVersion: 1,
      sourceWorldId: state.layout.worldId,
      gameMode: "unknown",
      cellSize: state.layout.cellSize,
      bounds: { ...state.layout.bounds },
      cells: state.layout.cells.map((cell) => ({
        x: cell.x,
        y: cell.y,
        tileUuid: String(cell.uuid || cell.tileKey),
        rotation: cell.rotation,
        xOffset: cell.xOffset,
        yOffset: cell.yOffset,
        flags: cell.flags
      }))
    };
  }

  function notifyProfileContextChanged() {
    const context = layoutProfileContext();
    if (context) {
      dispatchPersistenceEvent("sm-minimap:profile-context-changed", context);
    }
  }

  function normalizedFogCells(cells) {
    if (!state.layout || !cells || typeof cells[Symbol.iterator] !== "function") {
      return [];
    }
    const unique = new Map();
    for (const cell of cells) {
      let x;
      let y;
      if (typeof cell === "string") {
        const match = /^(-?\d+),(-?\d+)$/.exec(cell);
        if (!match) continue;
        x = Number(match[1]);
        y = Number(match[2]);
      } else if (cell && typeof cell === "object") {
        x = Number(cell.x == null ? cell.cellX : cell.x);
        y = Number(cell.y == null ? cell.cellY : cell.y);
      }
      if (!Number.isInteger(x) || !Number.isInteger(y)) continue;
      const key = Core.cellKey(x, y);
      if (state.layout.cellsByKey.has(key)) {
        unique.set(key, { x, y });
      }
    }
    return Array.from(unique.values());
  }

  function notifyFogDelta(cells) {
    const normalized = normalizedFogCells(cells);
    if (!normalized.length || persistenceIsSuppressed()) {
      return;
    }
    if (isTauriHost && state.profileHydrationStatus !== "ready") {
      normalized.forEach((cell) => state.pendingVisited.add(Core.cellKey(cell.x, cell.y)));
    }
    dispatchPersistenceEvent("sm-minimap:fog-delta", {
      schemaVersion: 1,
      sourceWorldId: state.layout.worldId,
      profileKey: state.activeProfileKey,
      cells: normalized
    });
  }

  function localMarkerPayload() {
    return state.localMarkers.map(({ id, cellX, cellY, kind, label, createdAt }) => ({
      id,
      cellX,
      cellY,
      kind,
      label,
      createdAt
    }));
  }

  function notifyLocalMarkersReplaced() {
    if (!state.layout) {
      return;
    }
    dispatchPersistenceEvent("sm-minimap:local-markers-replaced", {
      schemaVersion: 1,
      sourceWorldId: state.layout.worldId,
      profileKey: state.activeProfileKey,
      markers: localMarkerPayload()
    });
  }

  function profileSettingsPayload() {
    return {
      poiCategories: poiCategoryOrder.filter((category) => state.poiEnabled.has(category)),
      fogEnabled: state.fogEnabled
    };
  }

  function notifySettingsChanged() {
    if (!state.layout) {
      return;
    }
    dispatchPersistenceEvent("sm-minimap:settings-changed", {
      schemaVersion: 1,
      sourceWorldId: state.layout.worldId,
      profileKey: state.activeProfileKey,
      settings: profileSettingsPayload()
    });
  }

  function clonePersistentPayload(value) {
    if (value == null) {
      return null;
    }
    return typeof structuredClone === "function"
      ? structuredClone(value)
      : JSON.parse(JSON.stringify(value));
  }

  function setActiveRoute(route) {
    state.activeRoute = clonePersistentPayload(route);
    if (!state.layout) {
      return;
    }
    dispatchPersistenceEvent("sm-minimap:active-route-changed", {
      schemaVersion: 1,
      sourceWorldId: state.layout.worldId,
      profileKey: state.activeProfileKey,
      route: clonePersistentPayload(state.activeRoute)
    });
  }

  function writeTrailBatch(batch) {
    if (!state.layout || !batch || typeof batch !== "object") {
      return;
    }
    dispatchPersistenceEvent("sm-minimap:trail-batch", {
      schemaVersion: 1,
      sourceWorldId: state.layout.worldId,
      profileKey: state.activeProfileKey,
      batch: clonePersistentPayload(batch)
    });
  }

  function localStorageKey() {
    const worldId = state.layout ? state.layout.worldId : "unknown-world";
    return `sm-minimap:markers:${worldId}`;
  }

  function visitedStorageKey(worldId) {
    const id = worldId || (state.layout ? state.layout.worldId : "unknown-world");
    return `sm-minimap:visited:${id}`;
  }

  function loadLocalVisited() {
    if (!state.layout || isTauriHost) {
      return 0;
    }
    let added = 0;
    try {
      const saved = window.localStorage.getItem(visitedStorageKey());
      if (!saved) return 0;
      const normalized = Core.normalizeVisited(JSON.parse(saved));
      normalized.keys.forEach((key) => {
        if (state.layout.cellsByKey.has(key) && !state.visited.has(key)) {
          state.visited.add(key);
          added += 1;
        }
      });
    } catch (error) {
      showToast(`Не удалось прочитать исследованные клетки: ${error.message}`, true);
    }
    return added;
  }

  function scheduleVisitedSave() {
    if (!state.layout || persistenceIsSuppressed() || isTauriHost) {
      return;
    }
    window.clearTimeout(state.visitedSaveTimer);
    const worldId = state.layout.worldId;
    const payload = {
      schemaVersion: 1,
      worldId,
      visited: Array.from(state.visited)
    };
    state.visitedSaveTimer = window.setTimeout(() => {
      try {
        window.localStorage.setItem(visitedStorageKey(worldId), JSON.stringify(payload));
      } catch (error) {
        showToast(`Не удалось сохранить исследованные клетки: ${error.message}`, true);
      }
    }, 180);
  }

  function mergeSharedVisited(payload, expectedWorldId) {
    if (!state.layout || state.layout.worldId !== expectedWorldId) {
      return 0;
    }
    const normalized = Core.normalizeVisited(payload);
    if (normalized.worldId && normalized.worldId !== expectedWorldId) {
      return 0;
    }
    const reconciled = Core.reconcileVisitedKeys(state.visited, normalized.keys, {
      authoritative: payload && payload.authoritative === true,
      validKeys: state.layout.cellsByKey
    });
    if (reconciled.changed) {
      const addedCells = Array.from(reconciled.keys).filter((key) => !state.visited.has(key));
      state.visited = reconciled.keys;
      invalidateStaticFrame();
      scheduleVisitedSave();
      notifyFogDelta(addedCells);
      updateSummary();
      scheduleRender();
    }
    return reconciled.added;
  }

  async function pollSharedVisited() {
    if (
      state.sharedVisitedPollInFlight ||
      !state.layout ||
      document.hidden ||
      !/^https?:$/.test(window.location.protocol)
    ) {
      return;
    }
    const worldId = state.layout.worldId;
    state.sharedVisitedPollInFlight = true;
    try {
      const response = await window.fetch(
        `/api/visited?worldId=${encodeURIComponent(worldId)}`,
        {
          cache: "no-store",
          headers: { Accept: "application/json" }
        }
      );
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}`);
      }
      mergeSharedVisited(await response.json(), worldId);
    } catch {
      // localStorage remains the offline fallback when shared fog is unavailable.
    } finally {
      state.sharedVisitedPollInFlight = false;
    }
  }

  function revealAroundPlayers(players) {
    if (!state.layout) {
      return 0;
    }
    const discovered = Core.newlyRevealedCells(players, {
      cellSize: state.layout.cellSize,
      radius: autoRevealRadius,
      validKeys: state.layout.cellsByKey,
      visitedKeys: state.visited
    });
    discovered.forEach((cell) => state.visited.add(cell.key));
    if (discovered.length > 0) {
      invalidateStaticFrame();
      scheduleVisitedSave();
      notifyFogDelta(discovered);
    }
    return discovered.length;
  }

  /**
   * Marks every cell in the layout as visited.
   *
   * Nothing needs fetching: the whole layout and every tile image are already
   * local, so this only fills the visited set. Fog stays on, which keeps the
   * frontier shading and lets the map fill in for a friend through the usual
   * fog delta rather than being a private view-only toggle.
   */
  function revealAllCells() {
    if (!state.layout) {
      showToast("Карта ещё не загружена.", true);
      return 0;
    }
    const discovered = [];
    state.layout.cells.forEach((cell) => {
      if (!state.visited.has(cell.key)) {
        state.visited.add(cell.key);
        discovered.push(cell);
      }
    });
    if (discovered.length > 0) {
      invalidateStaticFrame();
      scheduleVisitedSave();
      // The storage layer rejects a fog merge over MAX_FOG_BATCH cells, and a
      // full reveal is several times that, so send it in batches.
      for (let index = 0; index < discovered.length; index += FOG_DELTA_BATCH) {
        notifyFogDelta(discovered.slice(index, index + FOG_DELTA_BATCH));
      }
      render();
    }
    showToast(
      discovered.length > 0
        ? `Открыто клеток: ${discovered.length}`
        : "Вся карта уже открыта.",
    );
    return discovered.length;
  }

  function loadLocalMarkers() {
    state.localMarkers = [];
    if (isTauriHost) {
      return;
    }
    try {
      const saved = window.localStorage.getItem(localStorageKey());
      if (!saved) return;
      const normalized = Core.normalizeMarkers(JSON.parse(saved));
      state.localMarkers = normalized.markers.map((marker) => ({ ...marker, local: true }));
    } catch (error) {
      showToast(`Не удалось прочитать локальные метки: ${error.message}`, true);
    }
  }

  function saveLocalMarkers() {
    if (persistenceIsSuppressed()) {
      return;
    }
    if (isTauriHost) {
      notifyLocalMarkersReplaced();
      return;
    }
    try {
      window.localStorage.setItem(localStorageKey(), JSON.stringify({
        schemaVersion: 1,
        worldId: state.layout ? state.layout.worldId : null,
        markers: state.localMarkers.map(({ id, cellX, cellY, kind, label, createdAt }) => ({
          id,
          cellX,
          cellY,
          kind,
          label,
          createdAt
        }))
      }));
    } catch (error) {
      showToast(`Не удалось сохранить локальные метки: ${error.message}`, true);
    }
  }

  function poiFilterStorageKey() {
    const worldId = state.layout ? state.layout.worldId : "unknown-world";
    return `sm-minimap:poi-filters:${worldId}`;
  }

  function loadPoiFilters() {
    state.poiEnabled = new Set(defaultPoiCategories);
    if (isTauriHost) {
      // In the app the profile store owns the fog preference and applySettings
      // restores it. Forcing it on here would discard an explicit opt-out every
      // time a layout or profile activates.
      elements.fogToggle.checked = state.fogEnabled;
      return;
    }
    state.fogEnabled = true;
    elements.fogToggle.checked = true;
    try {
      const saved = window.localStorage.getItem(poiFilterStorageKey());
      if (!saved) return;
      const payload = JSON.parse(saved);
      if (!payload || !Array.isArray(payload.enabled)) return;
      state.poiEnabled = new Set(
        payload.enabled
          .map((category) => String(category).toLowerCase())
          .filter((category) => poiCategories[category])
      );
      if (typeof payload.fogEnabled === "boolean") {
        state.fogEnabled = payload.fogEnabled;
        elements.fogToggle.checked = state.fogEnabled;
      }
    } catch (error) {
      showToast(`Не удалось прочитать фильтры POI: ${error.message}`, true);
    }
  }

  function savePoiFilters() {
    if (persistenceIsSuppressed()) {
      return;
    }
    if (isTauriHost) {
      notifySettingsChanged();
      return;
    }
    try {
      window.localStorage.setItem(poiFilterStorageKey(), JSON.stringify({
        schemaVersion: 1,
        worldId: state.layout ? state.layout.worldId : null,
        enabled: poiCategoryOrder.filter((category) => state.poiEnabled.has(category)),
        fogEnabled: state.fogEnabled
      }));
    } catch (error) {
      showToast(`Не удалось сохранить фильтры POI: ${error.message}`, true);
    }
  }

  function setPoiCategories(categories) {
    state.poiEnabled = new Set(
      Array.from(categories || []).filter((category) => poiCategories[category])
    );
    savePoiFilters();
    renderPoiFilters();
    invalidateStaticFrame();
    updateSummary();
    scheduleRender();
  }

  function focusPoi(record) {
    const point = record?.representative;
    if (!point) return;
    state.poiTargetId = record.id;
    state.poiTargetCellKeys = new Set(
      (record.cells || []).map((cell) => cell.key)
    );
    state.camera.x = point.x + 0.5;
    state.camera.y = point.y + 0.5;
    if (!state.expanded) setExpanded(true);
    invalidateStaticFrame();
    scheduleRender();
    showToast(`${record.name} · ${point.x} : ${point.y}`);
  }

  function renderPoiSearch() {
    const resultsElement = elements.poiSearchResults;
    if (!resultsElement) return;
    const query = state.poiSearchQuery.trim();
    resultsElement.replaceChildren();
    if (!query) {
      resultsElement.hidden = true;
      return;
    }

    const results = Core.searchPoiCatalog(state.poiCatalog, query, 10);
    resultsElement.hidden = false;
    if (!results.length) {
      const empty = document.createElement("p");
      empty.className = "poi-search-empty";
      empty.textContent = "Ничего не найдено";
      resultsElement.appendChild(empty);
      return;
    }

    results.forEach((record) => {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "poi-search-result";
      button.dataset.poiId = record.id;
      button.addEventListener("click", () => focusPoi(record));

      const definition = poiCategories[record.category] || poiCategories.landmark;
      const glyph = document.createElement("i");
      glyph.className = "poi-search-glyph";
      glyph.style.setProperty("--poi-color", definition.color);
      glyph.style.setProperty("--poi-fill", definition.fill);
      glyph.textContent = record.glyph || "•";

      const text = document.createElement("span");
      const name = document.createElement("strong");
      name.textContent = record.name;
      const detail = document.createElement("small");
      const coordinates = record.representative;
      detail.textContent = `${coordinates.x} : ${coordinates.y} · ${record.nameEn}`;
      text.append(name, detail);

      // Drawn larger than it is shown so the expanded panel stays crisp.
      const photo = poiPhotoThumbnail(record, 64);
      if (photo) {
        photo.className = "poi-search-photo";
        button.append(photo, text);
      } else {
        button.append(glyph, text);
      }
      resultsElement.appendChild(button);
    });
  }

  function poiCounts() {
    const counts = Object.fromEntries(poiCategoryOrder.map((category) => [category, 0]));
    if (state.layout) {
      state.layout.cells.forEach((cell) => {
        if (cell.poi) {
          const category = cell.poi.category || "landmark";
          counts[category] = (counts[category] || 0) + 1;
        }
      });
    }
    return counts;
  }

  // A thumbnail of the tile's photograph, or null when there is not one.
  //
  // The sweep's photographs are otherwise only visible by scrolling the map to
  // the right spot; in the search results they turn a list of names into a list
  // of places. Falls back to the category silhouette, which is what every tile
  // without a photograph still gets.
  function poiPhotoThumbnail(record, size = 40) {
    const uuid = String(record?.tileUuid || "").toLowerCase();
    if (!uuid || !tileAtlas.entries.size) return null;
    const entry = tileAtlas.entries.get(uuid);
    if (!entry || entry.topDownSourceKind !== "photo") return null;
    const image = atlasImage(entry);
    if (!image?.width) return null;

    const canvas = document.createElement("canvas");
    canvas.width = size;
    canvas.height = size;
    const context = canvas.getContext("2d");
    if (!context) return null;
    context.drawImage(image, 0, 0, image.width, image.height, 0, 0, size, size);
    return canvas;
  }

  /** Renders one category silhouette into a small canvas for the filter list. */
  function poiIconSwatch(category, color) {
    const size = 13;
    const ratio = Math.min(3, Math.max(1, window.devicePixelRatio || 1));
    const canvas = document.createElement("canvas");
    canvas.width = Math.round(size * ratio);
    canvas.height = Math.round(size * ratio);
    canvas.style.width = `${size}px`;
    canvas.style.height = `${size}px`;
    const ctx = canvas.getContext("2d");
    if (ctx) {
      ctx.scale(ratio, ratio);
      ctx.fillStyle = color;
      ctx.beginPath();
      (poiIcons[category] || poiIcons.landmark)(ctx, size / 2, size / 2, size / 2);
      ctx.fill();
    }
    return canvas;
  }

  function renderPoiFilters() {
    if (!elements.poiFilterList) return;
    const counts = poiCounts();
    elements.poiFilterList.replaceChildren();
    poiCategoryOrder.forEach((category) => {
      const definition = poiCategories[category];
      const label = document.createElement("label");
      label.className = `poi-filter${category === "schematic" ? " is-schematic" : ""}`;
      label.dataset.category = category;

      const input = document.createElement("input");
      input.type = "checkbox";
      input.checked = state.poiEnabled.has(category);
      input.setAttribute("aria-label", categoryLabel(definition));
      input.addEventListener("change", () => {
        if (input.checked) state.poiEnabled.add(category);
        else state.poiEnabled.delete(category);
        savePoiFilters();
        invalidateStaticFrame();
        updateSummary();
        scheduleRender();
      });

      const swatch = document.createElement("i");
      swatch.className = "poi-filter-swatch";
      swatch.style.setProperty("--poi-color", definition.color);
      swatch.style.setProperty("--poi-fill", definition.fill);
      // Draw the same silhouette the map uses, so the key is the legend rather
      // than a second thing to learn.
      swatch.replaceChildren(poiIconSwatch(category, definition.color));

      const text = document.createElement("span");
      const title = document.createElement("strong");
      title.textContent = categoryLabel(definition);
      const detail = document.createElement("small");
      detail.textContent = `${counts[category] || 0} на карте`;
      text.append(title, detail);

      label.append(input, swatch, text);
      elements.poiFilterList.appendChild(label);
    });
  }

  function showToast(message, isError) {
    window.clearTimeout(state.toastTimer);
    elements.toast.textContent = message;
    elements.toast.classList.toggle("is-error", Boolean(isError));
    elements.toast.classList.add("is-visible");
    state.toastTimer = window.setTimeout(() => elements.toast.classList.remove("is-visible"), 3000);
  }

  function updateSummary() {
    const player = state.lastLocalPlayer ||
      (state.telemetry && state.telemetry.player
        ? state.telemetry.player
        : { x: 0, y: 0 });
    const playerCell = state.layout
      ? Core.worldToCell(player, state.layout.cellSize)
      : { x: 0, y: 0 };

    if (staticFrame.summaryRevision !== staticFrame.revision) {
      const total = state.layout ? state.layout.cells.length : 0;
      const knownVisited = state.layout
        ? state.layout.cells.reduce((count, cell) => count + (state.visited.has(cell.key) ? 1 : 0), 0)
        : 0;
      const percent = total ? Math.round((knownVisited / total) * 100) : 0;
      const markers = allMarkers();
      elements.visitedMetric.textContent = `${percent}%`;
      elements.visitedDetail.textContent = text("SESSION_EXPLORED_OF")
        .replace("{visited}", knownVisited)
        .replace("{total}", total);
      elements.markersMetric.textContent = String(markers.length);
      if (elements.poiMetric) {
        const counts = poiCounts();
        const totalPoi = Object.values(counts).reduce((sum, count) => sum + count, 0);
        const visiblePoi = poiCategoryOrder.reduce(
          (sum, category) => sum + (state.poiEnabled.has(category) ? counts[category] : 0),
          0
        );
        elements.poiMetric.textContent = `${visiblePoi}/${totalPoi}`;
      }
      renderMarkerList(markers);
      staticFrame.summaryRevision = staticFrame.revision;
    }
    elements.cellLabel.textContent = `${playerCell.x} : ${playerCell.y}`;
    elements.positionLabel.textContent = `X ${player.x.toFixed(1)} · Y ${player.y.toFixed(1)}`;
    elements.worldChip.textContent = state.layout ? state.layout.worldId : "нет мира";
    if (state.telemetryStatus === "invalid") {
      elements.connectionLabel.textContent = "телеметрия отклонена";
    } else if (state.telemetryStatus === "unsupported") {
      elements.connectionLabel.textContent = "версия игры не поддерживается";
    } else if (state.telemetryStatus === "stale") {
      elements.connectionLabel.textContent = "последняя известная позиция";
    } else if (state.telemetryStatus === "waiting") {
      elements.connectionLabel.textContent = "ожидание телеметрии";
    } else if (state.telemetryLive) {
      const playerCount = telemetryPlayersInCurrentWorld().length;
      elements.connectionLabel.textContent =
        state.telemetryPollFailures > 8 || state.telemetryStale
          ? "поток координат ожидает игру или мир"
          : `координаты онлайн · игроков: ${playerCount}`;
    } else {
      elements.connectionLabel.textContent = state.layout
        ? "локальный набор данных загружен"
        : "ожидание layout";
    }
  }

  function renderMarkerList(markers) {
    elements.markerList.replaceChildren();
    if (!markers.length) {
      const empty = document.createElement("p");
      empty.className = "empty-state";
      empty.textContent = text("MARKERS_EMPTY");
      elements.markerList.appendChild(empty);
      return;
    }

    markers
      .slice()
      .sort((left, right) => (right.createdAt || "").localeCompare(left.createdAt || ""))
      .forEach((marker) => {
        const button = document.createElement("button");
        button.type = "button";
        button.className = "marker-item";
        button.innerHTML = `
          <i aria-hidden="true">×</i>
          <span>
            <strong></strong>
            <small></small>
          </span>
          <b></b>
        `;
        button.querySelector("strong").textContent = marker.label;
        button.querySelector("small").textContent = `${marker.cellX} : ${marker.cellY}`;
        button.querySelector("b").textContent = marker.local ? "МОЯ" : "ИМПОРТ";
        button.addEventListener("click", () => {
          if (!state.expanded) setExpanded(true);
          state.camera.x = marker.cellX + 0.5;
          state.camera.y = marker.cellY + 0.5;
          scheduleRender();
        });
        elements.markerList.appendChild(button);
      });
  }

  function centerOnPlayer() {
    const player = getPlayerGridPosition();
    state.camera.x = player.x;
    state.camera.y = player.y;
    scheduleRender();
  }

  function setExpanded(expanded) {
    const nextExpanded = Boolean(expanded);
    const changed = state.expanded !== nextExpanded;
    state.expanded = nextExpanded;
    elements.body.classList.toggle("map-expanded", state.expanded);
    elements.expandButton.setAttribute("aria-pressed", String(state.expanded));
    if (state.expanded) {
      centerOnPlayer();
    }
    window.setTimeout(resizeCanvas, 40);
    if (changed) {
      window.dispatchEvent(new CustomEvent("sm-minimap:mode-request", {
        detail: { expanded: state.expanded }
      }));
    }
  }

  function setMarkerMode(enabled) {
    state.markerMode = Boolean(enabled);
    elements.markerModeButton.setAttribute("aria-pressed", String(state.markerMode));
    elements.mapHint.textContent = state.markerMode
      ? "Нажмите на клетку — поставить или убрать ×"
      : state.expanded
        ? "Перетаскивание — панорама · колесо — масштаб"
        : "Двойной щелчок — поставить ×";
    positionHoverHighlight();
  }

  function toggleMarker(cellX, cellY) {
    if (!state.layout) return;
    const existingIndex = state.localMarkers.findIndex(
      (marker) => marker.cellX === cellX && marker.cellY === cellY
    );
    if (existingIndex >= 0) {
      state.localMarkers.splice(existingIndex, 1);
      showToast(`Метка ${cellX}:${cellY} удалена`);
    } else {
      state.localMarkers.push({
        id: `local-${Date.now()}-${cellX}-${cellY}`,
        cellX,
        cellY,
        kind: "x",
        label: `Посещено ${cellX}:${cellY}`,
        createdAt: new Date().toISOString(),
        local: true
      });
      showToast(`Локальная метка поставлена на ${cellX}:${cellY}`);
    }
    saveLocalMarkers();
    invalidateStaticFrame();
    updateSummary();
    scheduleRender();
  }

  function scheduleHoverPosition() {
    if (state.hoverPositionQueued) {
      return;
    }
    state.hoverPositionQueued = true;
    window.requestAnimationFrame(() => {
      state.hoverPositionQueued = false;
      if (elements.hoverCard.hidden) {
        return;
      }
      const cardWidth = 186;
      const cardHeight = 63;
      const left = Core.clamp(state.pointer.x + 17, 8, state.canvasWidth - cardWidth - 8);
      const top = Core.clamp(state.pointer.y + 17, 8, state.canvasHeight - cardHeight - 8);
      elements.hoverCard.style.transform = `translate3d(${left}px, ${top}px, 0)`;
    });
  }

  function positionHoverHighlight(view) {
    const cell = state.hoverCell && state.layout
      ? state.layout.cellsByKey.get(state.hoverCell.key)
      : null;
    if (!cell) {
      elements.hoverHighlight.hidden = true;
      return;
    }

    const currentView = view || {
      camera: getCamera(),
      scale: getScale()
    };
    const topLeft = gridToScreen(cell.x, cell.y + 1, currentView);
    const inset = 2;
    const size = Math.max(0, currentView.scale - inset * 2);
    elements.hoverHighlight.style.width = `${size}px`;
    elements.hoverHighlight.style.height = `${size}px`;
    elements.hoverHighlight.style.transform =
      `translate3d(${Math.floor(topLeft.x) + inset}px, ${Math.floor(topLeft.y) + inset}px, 0)`;
    elements.hoverHighlight.classList.toggle("is-marker-mode", state.markerMode);
    elements.hoverHighlight.hidden = false;
  }

  function updateHover(event) {
    const bounds = elements.canvas.getBoundingClientRect();
    state.pointer.x = event.clientX - bounds.left;
    state.pointer.y = event.clientY - bounds.top;
    const cellPosition = screenToCell(state.pointer.x, state.pointer.y);
    const cell = state.layout
      ? state.layout.cellsByKey.get(Core.cellKey(cellPosition.x, cellPosition.y))
      : null;
    const nextHoverCell = cell || {
      x: cellPosition.x,
      y: cellPosition.y,
      key: Core.cellKey(cellPosition.x, cellPosition.y)
    };
    const hoverChanged = !state.hoverCell || state.hoverCell.key !== nextHoverCell.key;
    state.hoverCell = nextHoverCell;

    if (!state.layout || !cell) {
      elements.hoverCard.hidden = true;
      elements.hoverHighlight.hidden = true;
      return;
    }

    if (hoverChanged) {
      const visible = !state.fogEnabled || isVisited(cell);
      const style = terrainStyle(cell.terrain);
      elements.hoverCoordinates.textContent = `${cell.x} : ${cell.y}`;
      elements.hoverTitle.textContent = visible
        ? cell.poi
          ? cell.poi.label
          : style.label
        : text("MAP_HOVER_UNEXPLORED");
      elements.hoverDetails.textContent = visible
        ? [
            cell.poi
              ? categoryLabel(poiCategories[cell.poi.category] || poiCategories.landmark)
              : style.label,
            cell.roads.length
              ? `${text("MAP_HOVER_ROADS")}: ${cell.roads.length}`
              : text("MAP_HOVER_NO_ROAD")
          ].join(" · ")
        : text("MAP_HOVER_FOGGED");
      positionHoverHighlight();
    }

    elements.hoverCard.hidden = false;
    scheduleHoverPosition();
  }

  function adjustZoom(multiplier, anchorX, anchorY) {
    if (!state.expanded) {
      if (multiplier > 1) setExpanded(true);
      return;
    }
    const before = screenToGrid(
      anchorX == null ? state.canvasWidth / 2 : anchorX,
      anchorY == null ? state.canvasHeight / 2 : anchorY
    );
    const minimumZoom = Core.minimumExpandedZoom({
      fogEnabled: state.fogEnabled,
      visitedKeys: state.visited,
      viewportWidth: state.canvasWidth,
      viewportHeight: state.canvasHeight,
      margin: 5,
      absoluteMinimum: 14,
      maximum: 116
    });
    state.camera.zoom = Core.clamp(getScale() * multiplier, minimumZoom, 116);
    const after = screenToGrid(
      anchorX == null ? state.canvasWidth / 2 : anchorX,
      anchorY == null ? state.canvasHeight / 2 : anchorY
    );
    state.camera.x += before.x - after.x;
    state.camera.y += before.y - after.y;
    scheduleRender();
  }

  function openDrawer(open) {
    const shouldOpen = Boolean(open);
    elements.dataDrawer.classList.toggle("is-open", shouldOpen);
    elements.dataDrawer.setAttribute("aria-hidden", String(!shouldOpen));
    elements.dataButton.setAttribute("aria-expanded", String(shouldOpen));
    if (shouldOpen) {
      window.setTimeout(() => elements.closeDrawerButton.focus(), 30);
    }
  }

  function ensureWorldMatch(data, kind) {
    if (
      state.layout &&
      data.worldId &&
      data.worldId !== state.layout.worldId
    ) {
      throw new TypeError(`${kind}: worldId отличается от текущего layout`);
    }
  }

  function applyLayout(payload, sourceName) {
    const layout = Core.normalizeLayout(payload);
    const previousWorldId = state.layout ? state.layout.worldId : null;
    const changedWorld = previousWorldId && previousWorldId !== layout.worldId;
    if (isTauriHost || !previousWorldId || changedWorld) {
      window.clearTimeout(state.visitedSaveTimer);
      state.visited.clear();
    }
    if (isTauriHost) {
      state.pendingVisited.clear();
      state.activeProfileKey = null;
      state.activeWorldFingerprint = null;
      state.activeRoute = null;
      state.recentTrail = null;
      state.profileHydrationStatus = "pending";
    }
    state.layout = layout;
    state.importedMarkers = [];
    state.poiCatalog = Core.buildPoiCatalog(layout);
    state.poiTargetId = null;
    state.poiTargetCellKeys.clear();
    if (changedWorld) {
      state.lastLocalPlayer = null;
    }
    loadLocalVisited();
    loadLocalMarkers();
    loadPoiFilters();
    renderPoiFilters();
    renderPoiSearch();
    notifyProfileContextChanged();
    if (!state.telemetry || (state.telemetry.worldId && state.telemetry.worldId !== layout.worldId)) {
      state.telemetry = {
        worldId: layout.worldId,
        timestamp: null,
        player: { x: 0, y: 0, z: 0, heading: 0 }
      };
    }
    revealAroundPlayers(
      state.telemetry.players
        ? telemetryPlayersInCurrentWorld()
        : state.lastLocalPlayer
          ? [state.lastLocalPlayer]
          : []
    );
    elements.layoutFileStatus.textContent =
      sourceName || text("DRAWER_CELLS").replace("{count}", layout.cells.length);
    elements.layoutFileStatus.classList.add("is-loaded");
    centerOnPlayer();
    invalidateStaticFrame();
    updateSummary();
    scheduleRender();
    if (layout.warnings.length) {
      showToast(layout.warnings[0], true);
    }
  }

  function applyTelemetry(payload, sourceName, options) {
    const telemetry = Core.normalizeTelemetry(payload);
    ensureWorldMatch(telemetry, "Telemetry");
    state.telemetry = telemetry;
    const localPlayer = telemetry.players.find((player) =>
      player.local &&
      player.active !== false &&
      player.hasCharacter !== false &&
      player.sameWorld !== false &&
      Number.isFinite(player.x) &&
      Number.isFinite(player.y) &&
      Number.isFinite(player.z)
    );
    if (localPlayer) {
      state.lastLocalPlayer = { ...localPlayer };
    }
    revealAroundPlayers(telemetryPlayersInCurrentWorld());
    if (options && options.live) {
      state.telemetryLive = true;
      state.telemetryStale = false;
      state.telemetryPollFailures = 0;
    }
    elements.telemetryFileStatus.textContent = sourceName || "обновлено";
    elements.telemetryFileStatus.classList.add("is-loaded");
    updateSummary();
    scheduleRender();
  }

  async function pollLiveTelemetry() {
    if (state.telemetryPollInFlight || document.hidden) {
      return;
    }
    state.telemetryPollInFlight = true;
    try {
      const response = await window.fetch("/api/telemetry", {
        cache: "no-store",
        headers: { Accept: "application/json" }
      });
      if (response.status === 204) {
        refreshTelemetryFreshness();
        return;
      }
      if (!response.ok) {
        throw new Error(`HTTP ${response.status}`);
      }
      const text = await response.text();
      const hadConnectionWarning = state.telemetryPollFailures > 8;
      state.telemetryPollFailures = 0;
      if (!text || text === state.telemetrySignature) {
        refreshTelemetryFreshness();
        if (state.telemetryLive && hadConnectionWarning) {
          updateSummary();
        }
        return;
      }
      const payload = JSON.parse(text);
      applyTelemetry(payload, "автообновление", { live: true });
      state.telemetrySignature = text;
    } catch {
      state.telemetryPollFailures += 1;
      if (state.telemetryLive && state.telemetryPollFailures === 9) {
        updateSummary();
      }
    } finally {
      state.telemetryPollInFlight = false;
    }
  }

  function refreshTelemetryFreshness() {
    if (!state.telemetryLive || !state.telemetry) {
      return;
    }
    const timestamp = Date.parse(state.telemetry.timestamp || "");
    const staleAfterMs = Number(state.telemetry.staleAfterMs) || 2000;
    const stale = !Number.isFinite(timestamp) ||
      Date.now() - timestamp > staleAfterMs;
    if (stale !== state.telemetryStale) {
      state.telemetryStale = stale;
      updateSummary();
    }
  }

  function startLiveTelemetry() {
    if (window.__TAURI__) {
      return;
    }
    if (!/^https?:$/.test(window.location.protocol)) {
      return;
    }
    pollLiveTelemetry();
    window.setInterval(pollLiveTelemetry, 250);
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) {
        pollLiveTelemetry();
      }
    });
  }

  function startSharedVisited() {
    if (isTauriHost || !/^https?:$/.test(window.location.protocol)) {
      return;
    }
    pollSharedVisited();
    window.setInterval(pollSharedVisited, 1250);
    document.addEventListener("visibilitychange", () => {
      if (!document.hidden) {
        pollSharedVisited();
      }
    });
  }

  function applyVisited(payload, sourceName, merge) {
    const visited = Core.normalizeVisited(payload);
    ensureWorldMatch(visited, "Visited");
    const previousVisited = state.visited;
    const nextVisited = merge ? new Set(state.visited) : new Set();
    visited.keys.forEach((key) => {
      if (!state.layout || state.layout.cellsByKey.has(key)) {
        nextVisited.add(key);
      }
    });
    const changed =
      nextVisited.size !== state.visited.size ||
      Array.from(nextVisited).some((key) => !state.visited.has(key));
    state.visited = nextVisited;
    elements.visitedFileStatus.textContent =
      sourceName || text("DRAWER_CELLS").replace("{count}", visited.keys.size);
    elements.visitedFileStatus.classList.add("is-loaded");
    if (changed) {
      invalidateStaticFrame();
      scheduleVisitedSave();
      notifyFogDelta(Array.from(nextVisited).filter((key) => !previousVisited.has(key)));
    }
    updateSummary();
    scheduleRender();
  }

  function applyMarkers(payload, sourceName) {
    const markers = Core.normalizeMarkers(payload);
    ensureWorldMatch(markers, "Markers");
    state.importedMarkers = markers.markers.map((marker) => ({ ...marker, local: false }));
    elements.markersFileStatus.textContent = sourceName || `${markers.markers.length} меток`;
    elements.markersFileStatus.classList.add("is-loaded");
    invalidateStaticFrame();
    updateSummary();
    scheduleRender();
  }

  function profileFogEntries(snapshot) {
    if (Array.isArray(snapshot.visited)) return snapshot.visited;
    if (Array.isArray(snapshot.fogCells)) return snapshot.fogCells;
    if (Array.isArray(snapshot.revealedCells)) return snapshot.revealedCells;
    if (snapshot.fog && typeof snapshot.fog === "object") {
      if (Array.isArray(snapshot.fog.revealedCells)) return snapshot.fog.revealedCells;
      if (Array.isArray(snapshot.fog.cells)) return snapshot.fog.cells;
      if (Array.isArray(snapshot.fog.visited)) return snapshot.fog.visited;
    }
    return [];
  }

  function normalizedProfileMarkers(entries, local) {
    if (!Array.isArray(entries)) {
      return [];
    }
    const markersById = new Map();
    entries.forEach((entry, index) => {
      if (!entry || typeof entry !== "object" || entry.kind === "scrapmap.marker-tombstone") {
        return;
      }
      const position = entry.position && typeof entry.position === "object"
        ? entry.position
        : {};
      const cell = position.cell && typeof position.cell === "object"
        ? position.cell
        : {};
      const cellX = Number(entry.cellX ?? entry.x ?? cell.x);
      const cellY = Number(entry.cellY ?? entry.y ?? cell.y);
      if (!Number.isInteger(cellX) || !Number.isInteger(cellY)) {
        return;
      }
      if (state.layout && !state.layout.cellsByKey.has(Core.cellKey(cellX, cellY))) {
        return;
      }
      const id = String(entry.id || `profile-${local ? "local" : "shared"}-${cellX}-${cellY}-${index}`);
      let markerKind = entry.icon;
      if (!markerKind) {
        markerKind = entry.kind === "scrapmap.marker" ? "x" : entry.kind;
      }
      markersById.set(id, {
        id,
        cellX,
        cellY,
        kind: String(markerKind || "x"),
        label: String(entry.label || `Метка ${cellX}:${cellY}`),
        createdAt: entry.createdAt || null,
        local
      });
    });
    return Array.from(markersById.values());
  }

  function splitProfileMarkers(snapshot) {
    const all = Array.isArray(snapshot.markers) ? snapshot.markers : [];
    const localEntries = Array.isArray(snapshot.localMarkers)
      ? snapshot.localMarkers
      : all.filter((marker) => marker && marker.scope !== "shared" && marker.local !== false);
    const sharedEntries = Array.isArray(snapshot.sharedMarkers)
      ? snapshot.sharedMarkers
      : all.filter((marker) => marker && (marker.scope === "shared" || marker.local === false));
    return {
      local: normalizedProfileMarkers(localEntries, true),
      shared: normalizedProfileMarkers(sharedEntries, false)
    };
  }

  function beginProfileHydration(sourceWorldId) {
    if (!state.layout || String(sourceWorldId || "") !== state.layout.worldId) {
      throw new TypeError("Нельзя начать загрузку профиля для другого sourceWorldId.");
    }
    state.profileHydrationStatus = "pending";
    state.pendingVisited.clear();
    return {
      sourceWorldId: state.layout.worldId,
      activeProfileKey: state.activeProfileKey,
      status: state.profileHydrationStatus
    };
  }

  function hydrateProfileState(snapshot, options) {
    if (!state.layout) {
      throw new TypeError("Нельзя восстановить профиль до загрузки layout.");
    }
    if (!snapshot || typeof snapshot !== "object") {
      throw new TypeError("Снимок профиля должен быть объектом.");
    }
    const profile = snapshot.profile && typeof snapshot.profile === "object"
      ? snapshot.profile
      : snapshot;
    const profileKey = String(profile.profileKey || "").trim();
    const worldFingerprint = String(profile.worldFingerprint || "").trim();
    const sourceWorldId = String(profile.sourceWorldId || snapshot.sourceWorldId || "").trim();
    if (!profileKey || !worldFingerprint || !sourceWorldId) {
      state.profileHydrationStatus = "error";
      throw new TypeError("Снимок профиля не содержит profileKey, worldFingerprint или sourceWorldId.");
    }
    if (sourceWorldId !== state.layout.worldId) {
      state.profileHydrationStatus = "error";
      throw new TypeError("Профиль относится к другому sourceWorldId.");
    }

    const hydrationOptions = options && typeof options === "object" ? options : {};
    const profileChanged = state.activeProfileKey !== profileKey;
    const pendingKeys = new Set(state.pendingVisited);
    const nextVisited = Core.normalizeVisited({
      worldId: sourceWorldId,
      visited: profileFogEntries(snapshot)
    }).keys;
    const validVisited = new Set(
      Array.from(nextVisited).filter((key) => state.layout.cellsByKey.has(key))
    );
    if (hydrationOptions.mergeFog === true && !profileChanged) {
      state.visited.forEach((key) => {
        if (state.layout.cellsByKey.has(key)) {
          validVisited.add(key);
        }
      });
    }
    if (hydrationOptions.preservePendingFog !== false) {
      pendingKeys.forEach((key) => {
        if (state.layout.cellsByKey.has(key)) {
          validVisited.add(key);
        }
      });
    }

    const markers = splitProfileMarkers(snapshot);
    const settings = snapshot.settings && typeof snapshot.settings === "object"
      ? snapshot.settings
      : {};
    const categories = Array.isArray(settings.poiCategories)
      ? settings.poiCategories
      : Array.isArray(settings.enabled)
        ? settings.enabled
        : poiCategoryOrder;
    const nextPoiEnabled = new Set(
      categories
        .map((category) => String(category).toLowerCase())
        .filter((category) => poiCategories[category])
    );
    const nextFogEnabled = typeof settings.fogEnabled === "boolean"
      ? settings.fogEnabled
      : true;

    withPersistenceSuppressed(() => {
      state.visited = validVisited;
      state.localMarkers = markers.local;
      state.importedMarkers = markers.shared;
      state.poiEnabled = nextPoiEnabled;
      state.fogEnabled = nextFogEnabled;
      state.activeProfileKey = profileKey;
      state.activeWorldFingerprint = worldFingerprint;
      state.activeRoute = clonePersistentPayload(snapshot.activeRoute);
      state.recentTrail = clonePersistentPayload(snapshot.recentTrail);
      state.profileHydrationStatus = "ready";
      state.pendingVisited.clear();
      elements.fogToggle.checked = state.fogEnabled;
      renderPoiFilters();
      invalidateStaticFrame();
      updateSummary();
      scheduleRender();
    });

    if (hydrationOptions.preservePendingFog !== false && pendingKeys.size) {
      notifyFogDelta(pendingKeys);
    }
    return {
      profileKey: state.activeProfileKey,
      worldFingerprint: state.activeWorldFingerprint,
      sourceWorldId: state.layout.worldId,
      status: state.profileHydrationStatus
    };
  }

  function applyBundle(bundle, sourceName) {
    if (!bundle || typeof bundle !== "object") {
      throw new TypeError("Bundle должен быть объектом.");
    }
    if (bundle.layout) applyLayout(bundle.layout, sourceName);
    if (bundle.telemetry) applyTelemetry(bundle.telemetry, sourceName);
    if (bundle.visited) applyVisited(bundle.visited, sourceName, true);
    if (bundle.markers) applyMarkers(bundle.markers, sourceName);
    if (!bundle.layout && !bundle.telemetry && !bundle.visited && !bundle.markers) {
      throw new TypeError("В bundle не найдены layout, telemetry, visited или markers.");
    }
  }

  async function parseFile(file) {
    const text = await file.text();
    try {
      return JSON.parse(text);
    } catch (error) {
      throw new SyntaxError(`${file.name}: некорректный JSON (${error.message}).`);
    }
  }

  async function loadFiles(files, forcedKind) {
    let loaded = 0;
    for (const file of Array.from(files || [])) {
      try {
        const payload = await parseFile(file);
        const kind = forcedKind || Core.classifyPayload(payload, file.name);
        if (kind === "bundle") applyBundle(payload, file.name);
        else if (kind === "layout") applyLayout(payload, file.name);
        else if (kind === "telemetry") applyTelemetry(payload, file.name);
        else if (kind === "visited") applyVisited(payload, file.name, true);
        else if (kind === "markers") applyMarkers(payload, file.name);
        else throw new TypeError(`${file.name}: тип данных не распознан.`);
        loaded += 1;
      } catch (error) {
        showToast(error.message, true);
      }
    }
    if (loaded) {
      showToast(`Загружено файлов: ${loaded}`);
    }
  }

  function downloadJson(fileName, payload) {
    const blob = new Blob([`${JSON.stringify(payload, null, 2)}\n`], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = fileName;
    document.body.appendChild(link);
    link.click();
    link.remove();
    URL.revokeObjectURL(url);
  }

  function buildDemoBundle() {
    const cells = [];
    const terrains = ["meadow", "meadow", "forest", "field", "desert", "industrial", "autumn"];
    const pois = new Map([
      ["-3,2", { kind: "warehouse", label: "Warehouse" }],
      ["0,0", { kind: "mechanic", label: "Mechanic Station" }],
      ["4,1", { kind: "packing", label: "Packing Station" }],
      ["2,-4", { kind: "ruin", label: "Ruined City" }],
      ["-4,-3", { kind: "camp", label: "Camp" }],
      ["1,4", { kind: "lab", label: "Grow Lab" }]
    ]);

    for (let y = -6; y <= 6; y += 1) {
      for (let x = -7; x <= 7; x += 1) {
        const distance = Math.hypot(x * 0.92, y);
        if (distance > 7.6 || (x === 6 && y < -2)) continue;
        const terrainIndex = Math.abs((x * 7 + y * 11 + Math.floor(distance)) % terrains.length);
        const roads = [];
        if (y === 0 && x >= -6 && x <= 5) {
          if (x > -6) roads.push("w");
          if (x < 5) roads.push("e");
        }
        if (x === 0 && y >= -5 && y <= 5) {
          if (y > -5) roads.push("s");
          if (y < 5) roads.push("n");
        }
        if (x === 3 && y >= 0 && y <= 3) {
          if (y > 0) roads.push("s");
          if (y < 3) roads.push("n");
        }
        if (y === 3 && x >= 0 && x <= 4) {
          if (x > 0) roads.push("w");
          if (x < 4) roads.push("e");
        }
        cells.push({
          x,
          y,
          uuid: `demo-${x + 8}-${y + 7}`,
          terrain: terrains[terrainIndex],
          rotation: Math.abs((x + y) % 4),
          roads,
          poi: pois.get(`${x},${y}`) || null
        });
      }
    }

    const visited = [];
    cells.forEach((cell) => {
      if (Math.hypot(cell.x - 0.2, cell.y + 0.1) < 4.25 || (cell.y === 0 && cell.x <= 4)) {
        visited.push({ x: cell.x, y: cell.y });
      }
    });

    return {
      layout: {
        schemaVersion: 1,
        worldId: "demo-overworld-667978921",
        seed: 667978921,
        cellSize: 64,
        bounds: { minX: -7, maxX: 7, minY: -6, maxY: 6 },
        cells
      },
      telemetry: {
        worldId: "demo-overworld-667978921",
        timestamp: new Date().toISOString(),
        player: { x: 54, y: -18, z: 12, heading: 34 }
      },
      visited: {
        worldId: "demo-overworld-667978921",
        visited
      },
      markers: {
        worldId: "demo-overworld-667978921",
        markers: [
          {
            id: "demo-marker-warehouse",
            cellX: -3,
            cellY: 2,
            kind: "x",
            label: "Вернуться за лутом",
            createdAt: "2026-07-26T18:20:00Z"
          }
        ]
      }
    };
  }

  function loadDemo() {
    state.visited.clear();
    applyBundle(buildDemoBundle(), "встроенное демо");
    elements.connectionLabel.textContent = "демонстрационные данные";
    showToast("Демонстрационный мир загружен");
  }

  elements.expandButton.addEventListener("click", () => setExpanded(!state.expanded));
  elements.dataButton.addEventListener("click", () => openDrawer(true));
  elements.closeDrawerButton.addEventListener("click", () => openDrawer(false));
  elements.drawerBackdrop.addEventListener("click", () => openDrawer(false));
  elements.loadDemoButton.addEventListener("click", loadDemo);
  elements.markerModeButton.addEventListener("click", () => setMarkerMode(!state.markerMode));
  elements.centerButton.addEventListener("click", centerOnPlayer);
  elements.zoomInButton.addEventListener("click", () => adjustZoom(1.22));
  elements.zoomOutButton.addEventListener("click", () => adjustZoom(1 / 1.22));

  elements.revealAllButton?.addEventListener("click", () => {
    revealAllCells();
  });

  elements.poiCaptureButton?.addEventListener("click", () => {
    // Preparing the sweep is a native concern: it writes a request the game
    // reads on its next world load.
    window.dispatchEvent(new CustomEvent("sm-minimap:poi-capture"));
  });

  // The compact map's placement is a property of this machine's screen and the
  // game's HUD, not of a world, so it lives in localStorage rather than the
  // per-profile store.
  const MINI_LAYOUT_STORAGE_KEY = "sm-minimap:mini-layout";

  function applyMiniLayout(persist) {
    const corner = Number(elements.miniCornerSelect?.value ?? 1);
    const size = Number(elements.miniSizeSelect?.value ?? 420);
    if (persist) {
      try {
        window.localStorage.setItem(
          MINI_LAYOUT_STORAGE_KEY,
          JSON.stringify({ corner, size })
        );
      } catch {
        // A full or disabled store only costs the preference, not the change.
      }
    }
    // Dispatched directly: this is native window placement, so it must not be
    // held back by profile hydration the way world data is.
    if (isTauriHost) {
      window.dispatchEvent(
        new CustomEvent("sm-minimap:mini-layout", { detail: { corner, size } })
      );
    }
  }

  function restoreMiniLayout() {
    try {
      const saved = JSON.parse(
        window.localStorage.getItem(MINI_LAYOUT_STORAGE_KEY) || "null"
      );
      if (saved && elements.miniCornerSelect && elements.miniSizeSelect) {
        elements.miniCornerSelect.value = String(saved.corner ?? 1);
        elements.miniSizeSelect.value = String(saved.size ?? 420);
      }
    } catch {
      // Fall back to the markup defaults.
    }
    applyMiniLayout(false);
  }

  elements.miniCornerSelect?.addEventListener("change", () => applyMiniLayout(true));
  elements.miniSizeSelect?.addEventListener("change", () => applyMiniLayout(true));

  // overlay-bridge.js loads after this file, so its listener does not exist
  // yet. Restoring now would dispatch into nothing and the saved corner would
  // silently never be applied on startup.
  if (document.readyState === "complete") {
    restoreMiniLayout();
  } else {
    window.addEventListener("load", restoreMiniLayout, { once: true });
  }

  elements.fogToggle.addEventListener("change", () => {
    state.fogEnabled = elements.fogToggle.checked;
    savePoiFilters();
    invalidateStaticFrame();
    updateSummary();
    scheduleRender();
  });

  elements.showAllPoiButton.addEventListener("click", () => {
    setPoiCategories(poiCategoryOrder);
  });

  elements.schematicOnlyButton.addEventListener("click", () => {
    setPoiCategories(["schematic"]);
    showToast("На карте оставлены только схемоботы");
  });

  elements.hideAllPoiButton.addEventListener("click", () => {
    setPoiCategories([]);
  });

  elements.poiSearchInput?.addEventListener("input", (event) => {
    state.poiSearchQuery = String(event.target.value || "");
    renderPoiSearch();
  });

  elements.exportMarkersButton.addEventListener("click", () => {
    const worldId = state.layout ? state.layout.worldId : "unknown-world";
    downloadJson(`markers-${worldId}.json`, {
      schemaVersion: 1,
      worldId,
      markers: allMarkers().map(({ id, cellX, cellY, kind, label, createdAt }) => ({
        id,
        cellX,
        cellY,
        kind,
        label,
        createdAt
      }))
    });
  });

  elements.clearMarkersButton.addEventListener("click", () => {
    if (!state.localMarkers.length) {
      showToast("Локальных меток для удаления нет");
      return;
    }
    state.localMarkers = [];
    saveLocalMarkers();
    invalidateStaticFrame();
    updateSummary();
    scheduleRender();
    showToast("Локальные метки этого мира удалены");
  });

  elements.layoutInput.addEventListener("change", (event) => loadFiles(event.target.files, "layout"));
  elements.telemetryInput.addEventListener("change", (event) => loadFiles(event.target.files, "telemetry"));
  elements.visitedInput.addEventListener("change", (event) => loadFiles(event.target.files, "visited"));
  elements.markersInput.addEventListener("change", (event) => loadFiles(event.target.files, "markers"));
  elements.bundleInput.addEventListener("change", (event) => loadFiles(event.target.files));

  ["dragenter", "dragover"].forEach((type) => {
    elements.bundleDrop.addEventListener(type, (event) => {
      event.preventDefault();
      elements.bundleDrop.classList.add("is-dragover");
    });
  });
  ["dragleave", "drop"].forEach((type) => {
    elements.bundleDrop.addEventListener(type, (event) => {
      event.preventDefault();
      elements.bundleDrop.classList.remove("is-dragover");
    });
  });
  elements.bundleDrop.addEventListener("drop", (event) => loadFiles(event.dataTransfer.files));

  elements.canvas.addEventListener("pointerdown", (event) => {
    const bounds = elements.canvas.getBoundingClientRect();
    if (state.markerMode) {
      return;
    }
    if (!state.expanded || event.button !== 0) {
      return;
    }
    elements.canvas.setPointerCapture(event.pointerId);
    state.drag = {
      pointerId: event.pointerId,
      x: event.clientX,
      y: event.clientY,
      cameraX: state.camera.x,
      cameraY: state.camera.y
    };
    elements.body.classList.add("is-dragging");
  });

  elements.canvas.addEventListener("click", (event) => {
    if (!state.markerMode || event.button !== 0 || event.detail > 1) {
      return;
    }
    const bounds = elements.canvas.getBoundingClientRect();
    const cell = screenToCell(event.clientX - bounds.left, event.clientY - bounds.top);
    toggleMarker(cell.x, cell.y);
  });

  elements.canvas.addEventListener("pointermove", (event) => {
    updateHover(event);
    if (!state.drag || state.drag.pointerId !== event.pointerId) return;
    const scale = getScale();
    state.camera.x = state.drag.cameraX - (event.clientX - state.drag.x) / scale;
    state.camera.y = state.drag.cameraY + (event.clientY - state.drag.y) / scale;
    scheduleRender();
  });

  function endDrag(event) {
    if (!state.drag || (event.pointerId != null && event.pointerId !== state.drag.pointerId)) return;
    state.drag = null;
    elements.body.classList.remove("is-dragging");
  }

  elements.canvas.addEventListener("pointerup", endDrag);
  elements.canvas.addEventListener("pointercancel", endDrag);
  elements.canvas.addEventListener("pointerleave", (event) => {
    elements.hoverCard.hidden = true;
    elements.hoverHighlight.hidden = true;
    state.hoverCell = null;
    endDrag(event);
  });

  elements.canvas.addEventListener("dblclick", (event) => {
    event.preventDefault();
    if (state.markerMode) {
      return;
    }
    const bounds = elements.canvas.getBoundingClientRect();
    const cell = screenToCell(event.clientX - bounds.left, event.clientY - bounds.top);
    toggleMarker(cell.x, cell.y);
  });

  elements.canvas.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    const bounds = elements.canvas.getBoundingClientRect();
    const cell = screenToCell(event.clientX - bounds.left, event.clientY - bounds.top);
    toggleMarker(cell.x, cell.y);
  });

  elements.canvas.addEventListener("wheel", (event) => {
    event.preventDefault();
    const bounds = elements.canvas.getBoundingClientRect();
    adjustZoom(event.deltaY < 0 ? 1.13 : 1 / 1.13, event.clientX - bounds.left, event.clientY - bounds.top);
  }, { passive: false });

  window.addEventListener("keydown", (event) => {
    if (elements.dataDrawer.classList.contains("is-open") && event.key === "Escape") {
      openDrawer(false);
      return;
    }
    if (event.target instanceof HTMLInputElement) return;
    if (event.key === "Escape" && state.expanded) setExpanded(false);
    else if (event.key.toLowerCase() === "f") setExpanded(!state.expanded);
    else if (event.key.toLowerCase() === "m") setMarkerMode(!state.markerMode);
    else if (event.key === "+" || event.key === "=") adjustZoom(1.18);
    else if (event.key === "-") adjustZoom(1 / 1.18);
    else if (event.key === "Home") centerOnPlayer();
    else if (state.expanded && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(event.key)) {
      event.preventDefault();
      const step = event.shiftKey ? 2 : 0.5;
      if (event.key === "ArrowLeft") state.camera.x -= step;
      if (event.key === "ArrowRight") state.camera.x += step;
      if (event.key === "ArrowUp") state.camera.y += step;
      if (event.key === "ArrowDown") state.camera.y -= step;
      scheduleRender();
    }
  });

  window.addEventListener("message", (event) => {
    const message = event.data;
    if (!message || message.type !== "sm-minimap:update") return;
    try {
      if (message.bundle) applyBundle(message.bundle, "postMessage");
      if (message.layout) applyLayout(message.layout, "postMessage");
      if (message.telemetry) applyTelemetry(message.telemetry, "postMessage");
      if (message.visited) applyVisited(message.visited, "postMessage", message.mergeVisited !== false);
      if (message.markers) applyMarkers(message.markers, "postMessage");
    } catch (error) {
      showToast(error.message, true);
    }
  });

  window.SMMinimap = Object.freeze({
    loadBundle(bundle) {
      applyBundle(bundle, "SMMinimap API");
    },
    setLayout(layout) {
      applyLayout(layout, "SMMinimap API");
    },
    updateTelemetry(telemetry, options) {
      applyTelemetry(telemetry, "встроенный канал", options);
    },
    updateVisited(visited, options) {
      applyVisited(visited, "SMMinimap API", !options || options.merge !== false);
    },
    setMarkers(markers) {
      applyMarkers(markers, "SMMinimap API");
    },
    setMarkerMode(enabled) {
      setMarkerMode(Boolean(enabled));
    },
    setPoiFilters(categories) {
      setPoiCategories(categories);
    },
    setTileAtlas(manifest, options) {
      return setTileAtlas(manifest, options);
    },
    getTileAtlasStatus() {
      return {
        fileCount: Array.isArray(tileAtlas.manifest?.entries)
          ? tileAtlas.manifest.entries.length
          : 0,
        uniqueTileIds: tileAtlas.entries.size,
        loadedImages: Array.from(tileAtlas.images.values()).filter(
          (record) => record.status === "ready",
        ).length,
        contentFingerprint: tileAtlas.manifest?.contentFingerprint || null,
      };
    },
    setActiveRoute(route) {
      setActiveRoute(route);
    },
    writeTrailBatch(batch) {
      writeTrailBatch(batch);
    },
    beginProfileHydration(sourceWorldId) {
      return beginProfileHydration(sourceWorldId);
    },
    hydrateProfileState(snapshot, options) {
      return hydrateProfileState(snapshot, options);
    },
    getProfileContext() {
      return layoutProfileContext();
    },
    getPersistenceStatus() {
      return {
        host: isTauriHost ? "tauri" : "browser",
        status: state.profileHydrationStatus,
        activeProfileKey: state.activeProfileKey,
        worldFingerprint: state.activeWorldFingerprint,
        sourceWorldId: state.layout ? state.layout.worldId : null,
        pendingFogCells: state.pendingVisited.size
      };
    },
    setTelemetryStatus(status) {
      const nextStatus = ["active", "invalid", "stale", "waiting", "unsupported"].includes(status)
        ? status
        : "waiting";
      state.telemetryStatus = nextStatus;
      state.telemetryStale = nextStatus !== "active";
      updateSummary();
    },
    getRenderStats() {
      return {
        ...state.renderStats,
        staticCacheValid: !staticFrame.dirty && Boolean(staticFrame.key)
      };
    },
    resetRenderStats() {
      Object.keys(state.renderStats).forEach((key) => {
        state.renderStats[key] = 0;
      });
    },
    getSnapshot() {
      return {
        worldId: state.layout ? state.layout.worldId : null,
        player: state.telemetry ? { ...state.telemetry.player } : null,
        players: state.telemetry
          ? (state.telemetry.players || [state.telemetry.player])
              .map((player) => ({ ...player }))
          : [],
        visited: Array.from(state.visited),
        markers: allMarkers().map((marker) => ({ ...marker })),
        localMarkers: state.localMarkers.map((marker) => ({ ...marker })),
        sharedMarkers: state.importedMarkers.map((marker) => ({ ...marker })),
        poiCategories: poiCategoryOrder.filter((category) => state.poiEnabled.has(category)),
        fogEnabled: state.fogEnabled,
        activeRoute: clonePersistentPayload(state.activeRoute),
        recentTrail: clonePersistentPayload(state.recentTrail),
        activeProfileKey: state.activeProfileKey,
        worldFingerprint: state.activeWorldFingerprint,
        profileHydrationStatus: state.profileHydrationStatus
      };
    },
    centerOnPlayer,
    setExpanded
  });

  const resizeObserver = new ResizeObserver(resizeCanvas);
  resizeObserver.observe(elements.canvas);
  const bootstrapAtlas = window.SMMinimapRuntimeAtlas;
  if (bootstrapAtlas?.manifest) {
    setTileAtlas(bootstrapAtlas.manifest, bootstrapAtlas.options);
  }
  const bootstrapBundle = window.SMMinimapBootstrapData;
  if (bootstrapBundle && typeof bootstrapBundle === "object" && bootstrapBundle.layout) {
    state.visited.clear();
    applyBundle(bootstrapBundle, "локальный runtime bundle");
    elements.connectionLabel.textContent = "локальный мир загружен";
    showToast("Карта локального мира загружена");
  } else {
    loadDemo();
  }
  startLiveTelemetry();
  startSharedVisited();
  resizeCanvas();
})();
