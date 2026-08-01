import { copyFile, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import path from "node:path";

function option(name) {
  const index = process.argv.indexOf(name);
  if (index < 0 || !process.argv[index + 1]) {
    throw new Error(`${name} is required`);
  }
  return path.resolve(process.argv[index + 1]);
}

const layoutPath = option("--layout");
const sourceRoot = option("--source");
const atlasRoot = option("--atlas");
const manifestPath = path.join(atlasRoot, "manifest.json");
const destinationRoot = path.join(atlasRoot, "tiles", "topdown");

const [layout, manifest, fileNames] = await Promise.all([
  readFile(layoutPath, "utf8").then(JSON.parse),
  readFile(manifestPath, "utf8").then(JSON.parse),
  readdir(path.join(sourceRoot, "tiles")),
]);

const availableIds = new Set(
  fileNames
    .filter((name) => /^\d+\.jpg$/i.test(name))
    .map((name) => Number.parseInt(path.basename(name, ".jpg"), 10)),
);
const legacyByUuid = new Map();
const layoutInfoByUuid = new Map();
for (const cell of layout.cells || []) {
  if (!cell?.uuid) continue;
  const uuid = String(cell.uuid).toLowerCase();
  if (Number.isFinite(cell.legacyId) && !legacyByUuid.has(uuid)) {
    legacyByUuid.set(uuid, Number(cell.legacyId));
  }
  if (!layoutInfoByUuid.has(uuid)) {
    layoutInfoByUuid.set(uuid, {
      path: String(cell.path || ""),
      poiCode: String(cell.poi?.code || ""),
      tileSize: Number(cell.tileSize || 1),
    });
  }
}

const poiAssetByCode = new Map([
  ["POI_MECHANICSTATION_MEDIUM", "mechanic_station.png"],
  ["POI_HIDEOUT_XL", "hideout.png"],
  ["POI_CAMP_LARGE", "camp_large.jpg"],
  ["POI_WAREHOUSE4_LARGE", "warehouse4.png"],
  ["POI_WAREHOUSE3_LARGE", "warehouse3_large.png"],
  ["POI_WAREHOUSE2_LARGE", "warehouse2.jpg"],
  ["POI_SILODISTRICT_XL", "silodistrict.jpg"],
  ["POI_RUINCITY_XL", "scrapcity.jpg"],
  ["POI_PACKINGSTATIONVEG_MEDIUM", "packing_veg.jpg"],
  ["POI_PACKINGSTATIONFRUIT_MEDIUM", "packing_fruit.jpg"],
  ["POI_CAPSULESCRAPYARD_MEDIUM", "capsule_scrapyard.jpg"],
  ["POI_BURNTFOREST_FARMBOTSCRAPYARD_LARGE", "burntforest_farmbot_scrapyard.jpg"],
  ["POI_LABYRINTH_MEDIUM", "labyrinth.jpg"],
  ["POI_BUILDAREA_MEDIUM", "buildarea.jpg"],
]);

function poiAsset(info) {
  if (info.path.toLowerCase().includes("crashedship")) {
    return {
      name: "crashed_ship.jpg",
      offsetX: info.path.toLowerCase().includes("_new.tile") ? -2 : 0,
      offsetY: info.path.toLowerCase().includes("_new.tile") ? -2 : 0,
    };
  }
  const name = poiAssetByCode.get(info.poiCode);
  return name ? { name, offsetX: 0, offsetY: 0 } : null;
}

await mkdir(destinationRoot, { recursive: true });
await mkdir(path.join(destinationRoot, "poi"), { recursive: true });
const copiedIds = new Set();
const copiedPoiAssets = new Set();
let matchedEntries = 0;
for (const entry of manifest.entries || []) {
  const uuid = String(entry.tileUuid || "").toLowerCase();
  const legacyId = legacyByUuid.get(uuid);
  if (availableIds.has(legacyId)) {
    const fileName = `${legacyId}.jpg`;
    entry.legacyId = legacyId;
    entry.topDownRelativePath = `topdown/${fileName}`;
    matchedEntries += 1;
    if (!copiedIds.has(legacyId)) {
      await copyFile(path.join(sourceRoot, "tiles", fileName), path.join(destinationRoot, fileName));
      copiedIds.add(legacyId);
    }
  }

  const info = layoutInfoByUuid.get(uuid);
  const asset = info && poiAsset(info);
  if (asset) {
    const assetName = asset.name;
    entry.poiOverlayRelativePath = `topdown/poi/${assetName}`;
    entry.poiOverlayTileSize = info.tileSize;
    entry.poiOverlayOffsetX = asset.offsetX;
    entry.poiOverlayOffsetY = asset.offsetY;
    if (!copiedPoiAssets.has(assetName)) {
      await copyFile(path.join(sourceRoot, assetName), path.join(destinationRoot, "poi", assetName));
      copiedPoiAssets.add(assetName);
    }
  }
}

manifest.topDownSource = {
  project: "the1killer/sm_overview",
  license: "CC BY-NC-SA 4.0",
  importedEntries: matchedEntries,
};
manifest.contentFingerprint = `${manifest.contentFingerprint || "smta1"}_topdown_${copiedIds.size}`;
await writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
await writeFile(
  path.join(atlasRoot, "TOPDOWN-ATTRIBUTION.txt"),
  [
    "Top-down tile images: the1killer/sm_overview",
    "https://github.com/the1killer/sm_overview",
    "License stated by the project: CC BY-NC-SA 4.0",
    "Imported into this local cache; not committed to ScrapMap.",
    "",
  ].join("\n"),
  "utf8",
);

console.log(JSON.stringify({
  matchedEntries,
  copiedFiles: copiedIds.size,
  copiedPoiAssets: copiedPoiAssets.size,
}, null, 2));
