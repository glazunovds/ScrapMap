(function exposeMapCore(root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  }
  if (root) {
    root.SMMapCore = api;
  }
})(typeof globalThis !== "undefined" ? globalThis : this, function createMapCore() {
  "use strict";

  const DIRECTIONS = ["n", "e", "s", "w"];
  const ROAD_BITS = { n: 1, e: 2, s: 4, w: 8 };
  const POI_TYPE_CATALOG = Object.freeze({
    schematic: {
      name: "Схемобот",
      nameEn: "Schematicbot",
      aliases: ["схема", "рецепт", "schematic", "recipe"],
      glyph: "S"
    },
    mechanic: {
      name: "Механик",
      nameEn: "Mechanic",
      aliases: ["станция механика", "mechanic station"],
      glyph: "M"
    },
    packing: {
      name: "Упаковочная станция",
      nameEn: "Packing station",
      aliases: ["упаковка", "packing"],
      glyph: "P"
    },
    warehouse: {
      name: "Склад",
      nameEn: "Warehouse",
      aliases: ["storage", "склад"],
      glyph: "W"
    },
    camp: {
      name: "Кемпинг-спот",
      nameEn: "Camp spot",
      aliases: ["лагерь", "camp"],
      glyph: "C"
    },
    quest: {
      name: "Квестовое место",
      nameEn: "Quest location",
      aliases: ["квест", "quest"],
      glyph: "!"
    },
    lab: {
      name: "Лаборатория",
      nameEn: "Laboratory",
      aliases: ["лаборатория", "lab"],
      glyph: "L"
    },
    ruin: {
      name: "Руины",
      nameEn: "Ruins",
      aliases: ["руины", "ruin"],
      glyph: "R"
    },
    dungeon: {
      name: "Подземелье",
      nameEn: "Dungeon",
      aliases: ["данж", "underground", "dungeon"],
      glyph: "D"
    }
  });

  function finiteNumber(value, fallback) {
    const number = Number(value);
    return Number.isFinite(number) ? number : fallback;
  }

  function integer(value, fallback) {
    const number = finiteNumber(value, fallback);
    return Number.isFinite(number) ? Math.trunc(number) : fallback;
  }

  function clamp(value, minimum, maximum) {
    return Math.min(maximum, Math.max(minimum, value));
  }

  function cellKey(x, y) {
    return `${integer(x, 0)},${integer(y, 0)}`;
  }

  function cellsInRadius(centerX, centerY, radius) {
    const x = integer(centerX, 0);
    const y = integer(centerY, 0);
    const safeRadius = Math.max(0, integer(radius, 0));
    const cells = [];
    for (let offsetY = -safeRadius; offsetY <= safeRadius; offsetY += 1) {
      for (let offsetX = -safeRadius; offsetX <= safeRadius; offsetX += 1) {
        if (offsetX * offsetX + offsetY * offsetY <= safeRadius * safeRadius) {
          cells.push({
            x: x + offsetX,
            y: y + offsetY,
            key: cellKey(x + offsetX, y + offsetY)
          });
        }
      }
    }
    return cells;
  }

  function newlyRevealedCells(players, options) {
    const source = options && typeof options === "object" ? options : {};
    const cellSize = Math.max(0.0001, finiteNumber(source.cellSize, 64));
    const radius = Math.max(0, integer(source.radius, 2));
    const validKeys = source.validKeys && typeof source.validKeys.has === "function"
      ? source.validKeys
      : null;
    const visitedKeys = source.visitedKeys && typeof source.visitedKeys.has === "function"
      ? source.visitedKeys
      : new Set();
    const discovered = new Map();
    (Array.isArray(players) ? players : []).forEach((player) => {
      if (
        !player ||
        player.active === false ||
        player.hasCharacter === false ||
        player.sameWorld === false ||
        !Number.isFinite(Number(player.x)) ||
        !Number.isFinite(Number(player.y))
      ) {
        return;
      }
      const center = worldToCell(player, cellSize);
      cellsInRadius(center.x, center.y, radius).forEach((cell) => {
        if (
          (!validKeys || validKeys.has(cell.key)) &&
          !visitedKeys.has(cell.key) &&
          !discovered.has(cell.key)
        ) {
          discovered.set(cell.key, cell);
        }
      });
    });
    return Array.from(discovered.values());
  }

  function reconcileVisitedKeys(currentKeys, incomingKeys, options) {
    const source = options && typeof options === "object" ? options : {};
    const validKeys = source.validKeys && typeof source.validKeys.has === "function"
      ? source.validKeys
      : null;
    const next = source.authoritative === true
      ? new Set()
      : new Set(currentKeys && typeof currentKeys[Symbol.iterator] === "function" ? currentKeys : []);
    if (incomingKeys && typeof incomingKeys[Symbol.iterator] === "function") {
      for (const key of incomingKeys) {
        const normalizedKey = String(key);
        if (!validKeys || validKeys.has(normalizedKey)) {
          next.add(normalizedKey);
        }
      }
    }
    const current = currentKeys && typeof currentKeys.has === "function"
      ? currentKeys
      : new Set();
    let added = 0;
    let removed = 0;
    next.forEach((key) => {
      if (!current.has(key)) added += 1;
    });
    current.forEach((key) => {
      if (!next.has(key)) removed += 1;
    });
    return {
      keys: next,
      added,
      removed,
      changed: added > 0 || removed > 0
    };
  }

  function minimumExpandedZoom(options) {
    const source = options && typeof options === "object" ? options : {};
    const absoluteMinimum = Math.max(0.0001, finiteNumber(source.absoluteMinimum, 14));
    const maximum = Math.max(absoluteMinimum, finiteNumber(source.maximum, 116));
    if (source.fogEnabled === false) {
      return absoluteMinimum;
    }

    const keys = source.visitedKeys && typeof source.visitedKeys[Symbol.iterator] === "function"
      ? source.visitedKeys
      : [];
    let minX = Infinity;
    let maxX = -Infinity;
    let minY = Infinity;
    let maxY = -Infinity;
    for (const value of keys) {
      const parts = String(value).split(",");
      const x = Number(parts[0]);
      const y = Number(parts[1]);
      if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
      minX = Math.min(minX, x);
      maxX = Math.max(maxX, x);
      minY = Math.min(minY, y);
      maxY = Math.max(maxY, y);
    }
    if (!Number.isFinite(minX)) {
      return maximum;
    }

    const margin = Math.max(0, finiteNumber(source.margin, 5));
    const widthInCells = Math.max(1, maxX - minX + 1 + margin * 2);
    const heightInCells = Math.max(1, maxY - minY + 1 + margin * 2);
    const viewportWidth = Math.max(1, finiteNumber(source.viewportWidth, 1));
    const viewportHeight = Math.max(1, finiteNumber(source.viewportHeight, 1));
    const fitZoom = Math.min(
      viewportWidth / widthInCells,
      viewportHeight / heightInCells
    );
    return clamp(fitZoom, absoluteMinimum, maximum);
  }

  function rectanglesOverlap(left, right, padding) {
    const gap = Math.max(0, finiteNumber(padding, 0));
    return !(
      left.x + left.width + gap <= right.x ||
      right.x + right.width + gap <= left.x ||
      left.y + left.height + gap <= right.y ||
      right.y + right.height + gap <= left.y
    );
  }

  function chooseLabelRect(candidates, occupied, bounds) {
    const safeBounds = bounds && typeof bounds === "object"
      ? bounds
      : { left: 0, top: 0, right: Infinity, bottom: Infinity };
    const blockers = Array.isArray(occupied) ? occupied : [];
    const normalized = (Array.isArray(candidates) ? candidates : []).map((candidate) => {
      const width = Math.max(1, finiteNumber(candidate.width, 1));
      const height = Math.max(1, finiteNumber(candidate.height, 1));
      const maximumX = Math.max(
        finiteNumber(safeBounds.left, 0),
        finiteNumber(safeBounds.right, Infinity) - width
      );
      const maximumY = Math.max(
        finiteNumber(safeBounds.top, 0),
        finiteNumber(safeBounds.bottom, Infinity) - height
      );
      return {
        x: clamp(
          finiteNumber(candidate.x, 0),
          finiteNumber(safeBounds.left, 0),
          maximumX
        ),
        y: clamp(
          finiteNumber(candidate.y, 0),
          finiteNumber(safeBounds.top, 0),
          maximumY
        ),
        width,
        height
      };
    });
    if (!normalized.length) return null;
    let best = normalized[0];
    let bestCollisionCount = Infinity;
    normalized.forEach((candidate) => {
      const collisionCount = blockers.reduce(
        (count, blocker) => count + (rectanglesOverlap(candidate, blocker, 3) ? 1 : 0),
        0
      );
      if (collisionCount < bestCollisionCount) {
        best = candidate;
        bestCollisionCount = collisionCount;
      }
    });
    return best;
  }

  function playerDisplayName(player) {
    const source = player && typeof player === "object" ? player : {};
    const name = String(source.name || "").trim();
    if (name) return name;
    const id = String(source.id || "").trim();
    return id ? `Игрок ${id}` : "Игрок";
  }

  function staticFrameKey(options) {
    const source = options && typeof options === "object" ? options : {};
    const camera = source.camera && typeof source.camera === "object"
      ? source.camera
      : {};
    return [
      integer(source.revision, 0),
      String(source.worldId || ""),
      source.expanded ? 1 : 0,
      Math.max(1, integer(source.width, 1)),
      Math.max(1, integer(source.height, 1)),
      Math.max(1, finiteNumber(source.pixelRatio, 1)),
      finiteNumber(camera.x, 0),
      finiteNumber(camera.y, 0),
      Math.max(0.0001, finiteNumber(source.scale, 1))
    ].join("|");
  }

  function visibleCellBounds(camera, viewportWidth, viewportHeight, scale, padding) {
    const safeCamera = camera && typeof camera === "object" ? camera : {};
    const cameraX = finiteNumber(safeCamera.x, 0);
    const cameraY = finiteNumber(safeCamera.y, 0);
    const safeScale = Math.max(0.0001, finiteNumber(scale, 1));
    const halfWidth = Math.max(0, finiteNumber(viewportWidth, 0)) / (safeScale * 2);
    const halfHeight = Math.max(0, finiteNumber(viewportHeight, 0)) / (safeScale * 2);
    const margin = Math.max(0, integer(padding, 0));

    return {
      minX: Math.floor(cameraX - halfWidth) - margin,
      maxX: Math.floor(cameraX + halfWidth) + margin,
      minY: Math.floor(cameraY - halfHeight) - margin,
      maxY: Math.floor(cameraY + halfHeight) + margin
    };
  }

  function forEachVisibleCell(layout, camera, viewportWidth, viewportHeight, scale, callback, padding) {
    if (!layout || typeof layout !== "object" || typeof callback !== "function") {
      return 0;
    }

    const viewBounds = visibleCellBounds(
      camera,
      viewportWidth,
      viewportHeight,
      scale,
      padding
    );
    const layoutBounds = layout.bounds || viewBounds;
    const minX = Math.max(viewBounds.minX, integer(layoutBounds.minX, viewBounds.minX));
    const maxX = Math.min(viewBounds.maxX, integer(layoutBounds.maxX, viewBounds.maxX));
    const minY = Math.max(viewBounds.minY, integer(layoutBounds.minY, viewBounds.minY));
    const maxY = Math.min(viewBounds.maxY, integer(layoutBounds.maxY, viewBounds.maxY));

    if (minX > maxX || minY > maxY) {
      return 0;
    }

    let count = 0;
    for (let y = minY; y <= maxY; y += 1) {
      const row = layout.cellsByY && layout.cellsByY.get(y);
      if (row) {
        for (let x = minX; x <= maxX; x += 1) {
          const cell = row.get(x);
          if (cell) {
            callback(cell);
            count += 1;
          }
        }
        continue;
      }

      for (let x = minX; x <= maxX; x += 1) {
        const cell = layout.cellsByKey && layout.cellsByKey.get(`${x},${y}`);
        if (cell) {
          callback(cell);
          count += 1;
        }
      }
    }

    return count;
  }

  function normalizeRotation(value) {
    const rotation = finiteNumber(value, 0);
    const quarterTurns = Math.abs(rotation) > 3
      ? Math.round(rotation / 90)
      : Math.round(rotation);
    return ((quarterTurns % 4) + 4) % 4;
  }

  function normalizeRoads(value) {
    if (Array.isArray(value)) {
      return Array.from(new Set(
        value
          .map((direction) => String(direction).trim().toLowerCase().charAt(0))
          .filter((direction) => DIRECTIONS.includes(direction))
      ));
    }

    if (typeof value === "number") {
      return DIRECTIONS.filter((direction) => (value & ROAD_BITS[direction]) !== 0);
    }

    if (value && typeof value === "object") {
      return DIRECTIONS.filter((direction) => Boolean(value[direction]));
    }

    return [];
  }

  function rotateRoads(roads, quarterTurns) {
    const turns = normalizeRotation(quarterTurns);
    return normalizeRoads(roads).map((direction) => {
      const index = DIRECTIONS.indexOf(direction);
      return DIRECTIONS[(index + turns) % DIRECTIONS.length];
    });
  }

  function normalizePoi(poi) {
    if (!poi) {
      return null;
    }

    if (typeof poi === "string") {
      return { kind: poi, label: poi };
    }

    if (typeof poi !== "object") {
      return null;
    }

    const kind = String(poi.kind || poi.type || poi.id || "poi");
    return {
      kind,
      label: String(poi.label || poi.name || kind),
      code: poi.code == null ? null : String(poi.code),
      category: poi.category == null ? null : String(poi.category).toLowerCase(),
      poiId: poi.poiId == null
        ? (poi.id == null ? null : String(poi.id))
        : String(poi.poiId),
      groupId: poi.groupId == null ? null : String(poi.groupId),
      displayName: poi.displayName == null
        ? String(poi.label || poi.name || kind)
        : String(poi.displayName),
      aliases: Array.isArray(poi.aliases)
        ? poi.aliases.map((alias) => String(alias)).filter(Boolean)
        : []
    };
  }

  function poiTypeDefinition(poi) {
    const kind = String(poi?.kind || poi?.type || "poi").toLowerCase();
    return POI_TYPE_CATALOG[kind] || {
      name: String(poi?.label || kind || "Точка интереса"),
      nameEn: kind || "POI",
      aliases: [],
      glyph: kind.charAt(0).toUpperCase() || "•"
    };
  }

  function buildPoiCatalog(layout) {
    const records = new Map();
    const cells = Array.isArray(layout?.cells) ? layout.cells : [];
    cells.forEach((cell) => {
      const poi = cell?.poi;
      if (!poi) return;
      const kind = String(poi.kind || poi.type || "poi").toLowerCase();
      const definition = poiTypeDefinition(poi);
      const explicitId = poi.poiId || poi.id || poi.code;
      const groupId = poi.groupId == null ? null : String(poi.groupId);
      const id = groupId
        ? `group:${groupId}`
        : explicitId
          ? `poi:${String(explicitId)}`
          : `cell:${kind}:${cell.x},${cell.y}`;
      let record = records.get(id);
      if (!record) {
        const displayName = String(
          poi.displayName || poi.label || definition.name || kind
        );
        record = {
          id,
          poiId: explicitId ? String(explicitId) : null,
          groupId,
          kind,
          category: String(poi.category || "landmark").toLowerCase(),
          name: displayName,
          nameEn: definition.nameEn,
          aliases: Array.from(new Set([
            ...definition.aliases,
            ...(Array.isArray(poi.aliases) ? poi.aliases : []),
            kind,
            poi.code || ""
          ].map(String).filter(Boolean))),
          glyph: definition.glyph,
          cells: [],
          representative: { x: cell.x, y: cell.y, key: cell.key }
        };
        record.searchText = [
          record.name,
          record.nameEn,
          record.kind,
          record.category,
          ...record.aliases,
          `${cell.x},${cell.y}`
        ].join(" ").toLocaleLowerCase();
        records.set(id, record);
      }
      record.cells.push({ x: cell.x, y: cell.y, key: cell.key });
      if (record.cells.length > 1) {
        record.searchText += ` ${cell.x},${cell.y}`;
      }
    });
    return Array.from(records.values());
  }

  function searchPoiCatalog(catalog, query, limit = 12) {
    const source = Array.isArray(catalog) ? catalog : [];
    const text = String(query || "").trim().toLocaleLowerCase();
    if (!text) return [];
    const terms = text.split(/\s+/).filter(Boolean);
    return source
      .map((record) => {
        const name = String(record?.name || "").toLocaleLowerCase();
        const aliases = Array.isArray(record?.aliases)
          ? record.aliases.map((alias) => String(alias).toLocaleLowerCase())
          : [];
        const searchable = String(record?.searchText || "").toLocaleLowerCase();
        if (!terms.every((term) => searchable.includes(term))) return null;
        let score = 10;
        if (name === text) score += 1000;
        else if (name.startsWith(text)) score += 700;
        else if (name.includes(text)) score += 400;
        if (aliases.some((alias) => alias === text)) score += 300;
        else if (aliases.some((alias) => alias.includes(text))) score += 160;
        if (searchable.includes(text)) score += 20;
        return { record, score };
      })
      .filter(Boolean)
      .sort((left, right) =>
        right.score - left.score ||
        String(left.record.name).localeCompare(String(right.record.name), "ru") ||
        String(left.record.id).localeCompare(String(right.record.id))
      )
      .slice(0, Math.max(1, integer(limit, 12)))
      .map(({ record }) => record);
  }

  function tileLabel(path) {
    const fileName = String(path || "")
      .replaceAll("\\", "/")
      .split("/")
      .pop()
      .replace(/\.tile$/i, "");
    return fileName.replaceAll("_", " ").replace(/\s+/g, " ").trim();
  }

  function inferPoi(rawCell) {
    if (!rawCell || typeof rawCell !== "object") {
      return null;
    }

    const xOffset = integer(rawCell.xOffset, 0);
    const yOffset = integer(rawCell.yOffset, 0);
    if (xOffset !== 0 || yOffset !== 0) {
      return null;
    }

    const path = String(rawCell.path || "").replaceAll("\\", "/");
    const lower = path.toLowerCase();
    if (lower.includes("schematicstation")) {
      return {
        kind: "schematic",
        label: "Схемобот — обмен схем на рецепты",
        code: "schematic-station",
        category: "schematic"
      };
    }

    if (
      lower.includes("/questtiles/builderquest_") ||
      lower.includes("ruinsquest")
    ) {
      return {
        kind: "quest",
        label: tileLabel(path) || "Квестовое место",
        code: null,
        category: "quest"
      };
    }

    return null;
  }

  function poiCategory(poi, path) {
    if (!poi) {
      return null;
    }
    if (poi.category) {
      return String(poi.category).toLowerCase();
    }

    const searchable = [
      poi.kind,
      poi.label,
      poi.code,
      path
    ].filter(Boolean).join(" ").toLowerCase();

    if (searchable.includes("schematic")) return "schematic";
    if (searchable.includes("camp")) return "camp";
    if (searchable.includes("warehouse")) return "warehouse";
    if (
      searchable.includes("mechanic") ||
      searchable.includes("packing") ||
      searchable.includes("hideout")
    ) {
      return "service";
    }
    if (
      searchable.includes("builderquest") ||
      searchable.includes("ruinsquest") ||
      searchable.includes("bunkinvestigationquest")
    ) {
      return "quest";
    }
    if (
      searchable.includes("minidungeon") ||
      searchable.includes("underground") ||
      searchable.includes("dungeon")
    ) {
      return "dungeon";
    }
    return "landmark";
  }

  function deriveBounds(cells) {
    if (!cells.length) {
      return { minX: 0, maxX: 0, minY: 0, maxY: 0 };
    }

    return cells.reduce((bounds, cell) => ({
      minX: Math.min(bounds.minX, cell.x),
      maxX: Math.max(bounds.maxX, cell.x),
      minY: Math.min(bounds.minY, cell.y),
      maxY: Math.max(bounds.maxY, cell.y)
    }), {
      minX: cells[0].x,
      maxX: cells[0].x,
      minY: cells[0].y,
      maxY: cells[0].y
    });
  }

  function normalizeBounds(bounds, cells) {
    const derived = deriveBounds(cells);
    if (!bounds || typeof bounds !== "object") {
      return derived;
    }

    return {
      minX: integer(bounds.minX, derived.minX),
      maxX: integer(bounds.maxX, derived.maxX),
      minY: integer(bounds.minY, derived.minY),
      maxY: integer(bounds.maxY, derived.maxY)
    };
  }

  function unwrap(payload, property) {
    if (payload && typeof payload === "object" && payload[property]) {
      return payload[property];
    }
    return payload;
  }

  function normalizeLayout(payload) {
    const source = unwrap(payload, "layout");
    if (!source || typeof source !== "object") {
      throw new TypeError("Layout должен быть JSON-объектом.");
    }
    if (!Array.isArray(source.cells)) {
      throw new TypeError("Layout должен содержать массив cells.");
    }

    const warnings = [];
    const seen = new Set();
    const cells = source.cells.map((rawCell, index) => {
      if (!rawCell || typeof rawCell !== "object") {
        throw new TypeError(`Некорректная клетка layout.cells[${index}].`);
      }

      const x = integer(rawCell.x, NaN);
      const y = integer(rawCell.y, NaN);
      if (!Number.isFinite(x) || !Number.isFinite(y)) {
        throw new TypeError(`У клетки layout.cells[${index}] отсутствуют целые x/y.`);
      }

      const key = cellKey(x, y);
      if (seen.has(key)) {
        warnings.push(`Повторная клетка ${key}; будет использована последняя.`);
      }
      seen.add(key);

      const tileKey = rawCell.uuid || rawCell.path || rawCell.tileKey || rawCell.tileId || rawCell.id || key;
      let normalizedPoi = normalizePoi(rawCell.poi) || inferPoi(rawCell);
      const path = rawCell.path == null ? null : String(rawCell.path);
      const xOffset = integer(rawCell.xOffset, 0);
      const yOffset = integer(rawCell.yOffset, 0);
      if (normalizedPoi && (xOffset !== 0 || yOffset !== 0)) {
        normalizedPoi = null;
      }
      if (
        normalizedPoi?.code === "POI_CRASHSITE_AREA" &&
        !String(path || "").toLowerCase().includes("crashedship")
      ) {
        normalizedPoi = null;
      }
      if (normalizedPoi) {
        normalizedPoi.category = poiCategory(normalizedPoi, path);
        if (normalizedPoi.groupId == null && Number(rawCell.groupId) > 0) {
          normalizedPoi.groupId = String(integer(rawCell.groupId, 0));
        }
      }
      return {
        x,
        y,
        key,
        uuid: rawCell.uuid == null ? null : String(rawCell.uuid),
        path,
        tileKey: String(tileKey),
        terrain: String(rawCell.terrain || rawCell.biome || "unknown").toLowerCase(),
        rotation: normalizeRotation(rawCell.rotation),
        roads: normalizeRoads(rawCell.roads == null ? rawCell.roadMask : rawCell.roads),
        poi: normalizedPoi,
        xOffset,
        yOffset,
        tileSize: Math.max(1, integer(rawCell.tileSize, 1)),
        groupId: rawCell.groupId == null ? null : integer(rawCell.groupId, null),
        flags: integer(rawCell.flags, 0)
      };
    });

    const cellsByKey = new Map();
    cells.forEach((cell) => cellsByKey.set(cell.key, cell));
    const uniqueCells = Array.from(cellsByKey.values());
    const cellsByY = new Map();
    uniqueCells.forEach((cell) => {
      let row = cellsByY.get(cell.y);
      if (!row) {
        row = new Map();
        cellsByY.set(cell.y, row);
      }
      row.set(cell.x, cell);
    });
    const cellSize = finiteNumber(source.cellSize, 64);
    if (!(cellSize > 0)) {
      throw new RangeError("layout.cellSize должен быть больше нуля.");
    }

    return {
      schemaVersion: integer(source.schemaVersion, 1),
      worldId: String(source.worldId || source.seed || "unknown-world"),
      seed: source.seed == null ? null : String(source.seed),
      cellSize,
      bounds: normalizeBounds(source.bounds, uniqueCells),
      cells: uniqueCells,
      cellsByKey,
      cellsByY,
      warnings
    };
  }

  function normalizeTelemetry(payload) {
    const source = unwrap(payload, "telemetry");
    if (!source || typeof source !== "object") {
      throw new TypeError("Telemetry должен быть JSON-объектом.");
    }

    const sourcePlayers = Array.isArray(source.players) ? source.players : [];
    const primarySource =
      source.player ||
      sourcePlayers.find((player) => player && (player.local || player.isLocal)) ||
      sourcePlayers[0] ||
      source;
    if (!primarySource || typeof primarySource !== "object") {
      throw new TypeError("Telemetry должен содержать объект player.");
    }

    const payloadWorldId = source.payloadWorldId == null
      ? null
      : String(source.payloadWorldId);
    const normalizePlayer = (player, index, localFallback) => {
      if (!player || typeof player !== "object") {
        return null;
      }
      const id = player.id == null ? null : String(player.id);
      const hasCoordinates = ["x", "y", "z"].every((field) =>
        Number.isFinite(Number(player[field]))
      );
      const fallbackName = localFallback
        ? "Вы"
        : id == null
          ? `Игрок ${index + 1}`
          : `Игрок ${id}`;
      return {
        id,
        name: String(player.name || player.playerName || fallbackName).slice(0, 80),
        local: player.local == null && player.isLocal == null
          ? Boolean(localFallback)
          : Boolean(player.local || player.isLocal),
        active: player.active !== false,
        hasCharacter: player.hasCharacter == null
          ? hasCoordinates
          : Boolean(player.hasCharacter),
        sameWorld: player.sameWorld == null ? true : Boolean(player.sameWorld),
        worldId: player.worldId == null
          ? player.payloadWorldId == null
            ? payloadWorldId
            : String(player.payloadWorldId)
          : String(player.worldId),
        x: finiteNumber(player.x, 0),
        y: finiteNumber(player.y, 0),
        z: finiteNumber(player.z, 0),
        heading: ((finiteNumber(player.heading, player.yaw || 0) % 360) + 360) % 360
      };
    };

    const primary = normalizePlayer(primarySource, 0, true);
    const players = sourcePlayers
      .map((player, index) => normalizePlayer(player, index, false))
      .filter(Boolean);
    let localPlayer =
      players.find((player) => player.local) ||
      (primary.id == null
        ? null
        : players.find((player) => player.id === primary.id)) ||
      primary;
    const primaryAlreadyPresent = players.some((player) =>
      primary.id == null
        ? player === localPlayer
        : player.id === primary.id
    );
    if (!primaryAlreadyPresent) {
      players.unshift(primary);
      localPlayer = primary;
    }
    players.forEach((player) => {
      player.local = player === localPlayer ||
        (localPlayer.id != null && player.id === localPlayer.id);
    });

    return {
      schemaVersion: integer(source.schemaVersion, 1),
      worldId: source.worldId == null ? null : String(source.worldId),
      payloadWorldId,
      timestamp: source.timestamp || null,
      staleAfterMs: clamp(finiteNumber(source.staleAfterMs, 2000), 500, 30000),
      player: localPlayer,
      players
    };
  }

  function normalizeVisited(payload) {
    const source = payload &&
      typeof payload === "object" &&
      !Array.isArray(payload) &&
      payload.visited &&
      !Array.isArray(payload.visited) &&
      typeof payload.visited === "object"
      ? payload.visited
      : payload;
    let entries = source;
    let worldId = null;

    if (source && typeof source === "object" && !Array.isArray(source)) {
      worldId = source.worldId == null ? null : String(source.worldId);
      entries = source.visited || source.cells || source.keys || [];
    }

    if (!Array.isArray(entries)) {
      throw new TypeError("Visited должен быть массивом или объектом с массивом visited.");
    }

    const keys = new Set();
    entries.forEach((entry) => {
      if (typeof entry === "string" && /^-?\d+,-?\d+$/.test(entry.trim())) {
        keys.add(entry.trim());
      } else if (Array.isArray(entry) && entry.length >= 2) {
        keys.add(cellKey(entry[0], entry[1]));
      } else if (entry && typeof entry === "object") {
        keys.add(cellKey(entry.x == null ? entry.cellX : entry.x, entry.y == null ? entry.cellY : entry.y));
      }
    });

    return { worldId, keys };
  }

  function normalizeMarkers(payload) {
    const source = payload &&
      typeof payload === "object" &&
      !Array.isArray(payload) &&
      payload.markers &&
      !Array.isArray(payload.markers) &&
      typeof payload.markers === "object"
      ? payload.markers
      : payload;
    let entries = source;
    let worldId = null;

    if (source && typeof source === "object" && !Array.isArray(source)) {
      worldId = source.worldId == null ? null : String(source.worldId);
      entries = source.markers || [];
    }

    if (!Array.isArray(entries)) {
      throw new TypeError("Markers должен быть массивом или объектом с массивом markers.");
    }

    const markers = entries
      .filter((entry) => entry && typeof entry === "object")
      .map((entry, index) => {
        const cellX = integer(entry.cellX == null ? entry.x : entry.cellX, NaN);
        const cellY = integer(entry.cellY == null ? entry.y : entry.cellY, NaN);
        if (!Number.isFinite(cellX) || !Number.isFinite(cellY)) {
          return null;
        }
        return {
          id: String(entry.id || `imported-${cellX}-${cellY}-${index}`),
          cellX,
          cellY,
          kind: String(entry.kind || "x"),
          label: String(entry.label || `Метка ${cellX}:${cellY}`),
          createdAt: entry.createdAt || null,
          local: Boolean(entry.local)
        };
      })
      .filter(Boolean);

    return { worldId, markers };
  }

  function worldToCell(position, cellSize) {
    const size = finiteNumber(cellSize, 64);
    return {
      x: Math.floor(finiteNumber(position.x, 0) / size),
      y: Math.floor(finiteNumber(position.y, 0) / size)
    };
  }

  function cellToWorld(cell, cellSize) {
    const size = finiteNumber(cellSize, 64);
    return {
      x: (finiteNumber(cell.x, 0) + 0.5) * size,
      y: (finiteNumber(cell.y, 0) + 0.5) * size
    };
  }

  function classifyPayload(payload, fileName) {
    const name = String(fileName || "").toLowerCase();
    if (payload && typeof payload === "object") {
      if (payload.layout || payload.telemetry || payload.visited || payload.markers) {
        const bundleKeys = ["layout", "telemetry", "visited", "markers"].filter((key) => payload[key]);
        if (bundleKeys.length > 1 || payload.layout) {
          return "bundle";
        }
      }
      if (Array.isArray(payload.cells) && (payload.cellSize || payload.bounds || payload.seed || payload.schemaVersion)) {
        return "layout";
      }
      if (payload.player || (Number.isFinite(Number(payload.x)) && Number.isFinite(Number(payload.y)) && payload.heading != null)) {
        return "telemetry";
      }
      if (Array.isArray(payload.visited) || Array.isArray(payload.keys)) {
        return "visited";
      }
      if (Array.isArray(payload.markers)) {
        return "markers";
      }
    }

    if (name.includes("layout")) return "layout";
    if (name.includes("telemetry")) return "telemetry";
    if (name.includes("visited")) return "visited";
    if (name.includes("marker")) return "markers";
    return "unknown";
  }

  return Object.freeze({
    DIRECTIONS,
    ROAD_BITS,
    clamp,
    cellKey,
    cellsInRadius,
    newlyRevealedCells,
    reconcileVisitedKeys,
    minimumExpandedZoom,
    rectanglesOverlap,
    chooseLabelRect,
    playerDisplayName,
    staticFrameKey,
    visibleCellBounds,
    forEachVisibleCell,
    normalizeRotation,
    normalizeRoads,
    rotateRoads,
    inferPoi,
    poiCategory,
    POI_TYPE_CATALOG,
    buildPoiCatalog,
    searchPoiCatalog,
    normalizeLayout,
    normalizeTelemetry,
    normalizeVisited,
    normalizeMarkers,
    worldToCell,
    cellToWorld,
    classifyPayload
  });
});
