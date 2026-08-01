import { createHash } from "node:crypto";
import { copyFile, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

export const TILE_ATLAS_SCHEMA_VERSION = 1;
export const TILE_ATLAS_SOURCE = "Survival/Terrain/Tiles";

const PNG_SIGNATURE = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const TILE_UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-5][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;

function parseOptionValue(argv, index, option) {
  const value = argv[index + 1];
  if (!value || value.startsWith("--")) {
    throw new Error(`${option} requires a value.`);
  }
  return value;
}

export function parseArguments(argv) {
  const options = {
    gameRoot: process.env.SCRAPMECHANIC_ROOT || null,
    output: null,
    copyTo: null,
    help: false,
  };

  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (argument === "--help" || argument === "-h") {
      options.help = true;
      continue;
    }
    if (argument === "--game-root") {
      options.gameRoot = parseOptionValue(argv, index, argument);
      index += 1;
      continue;
    }
    if (argument === "--output") {
      options.output = parseOptionValue(argv, index, argument);
      index += 1;
      continue;
    }
    if (argument === "--copy-to") {
      options.copyTo = parseOptionValue(argv, index, argument);
      index += 1;
      continue;
    }
    throw new Error(`Unknown option: ${argument}`);
  }

  return options;
}

async function listPngFiles(directory, relativeDirectory = "") {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];

  for (const entry of entries) {
    const absolutePath = path.join(directory, entry.name);
    const relativePath = path.posix.join(relativeDirectory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await listPngFiles(absolutePath, relativePath)));
    } else if (entry.isFile() && path.extname(entry.name).toLowerCase() === ".png") {
      files.push({ absolutePath, relativePath });
    }
  }

  return files;
}

export function readPngDimensions(buffer) {
  if (
    buffer.length < 24 ||
    !PNG_SIGNATURE.every((byte, index) => buffer[index] === byte) ||
    buffer.readUInt32BE(8) < 13 ||
    buffer.toString("ascii", 12, 16) !== "IHDR"
  ) {
    throw new Error("Invalid PNG header.");
  }

  const width = buffer.readUInt32BE(16);
  const height = buffer.readUInt32BE(20);
  if (width === 0 || height === 0) {
    throw new Error("PNG dimensions must be positive.");
  }
  return { width, height };
}

function sha256(buffer) {
  return createHash("sha256").update(buffer).digest("hex");
}

function tileUuidFromRelativePath(relativePath) {
  const tileUuid = path.posix.basename(relativePath, ".png").toLowerCase();
  if (!TILE_UUID_PATTERN.test(tileUuid)) {
    throw new Error(`Tile filename is not a UUID: ${relativePath}`);
  }
  return tileUuid;
}

function atlasFingerprint(entries) {
  return `smta1_${sha256(Buffer.from(JSON.stringify(entries), "utf8"))}`;
}

export async function scanTileAtlas(gameRoot) {
  if (!gameRoot) {
    throw new Error(
      "Game root is required. Pass --game-root or set SCRAPMECHANIC_ROOT.",
    );
  }

  const sourceRoot = path.resolve(gameRoot, ...TILE_ATLAS_SOURCE.split("/"));
  const files = (await listPngFiles(sourceRoot)).sort((left, right) =>
    left.relativePath.localeCompare(right.relativePath),
  );
  if (files.length === 0) {
    throw new Error(`No tile previews found under ${sourceRoot}.`);
  }

  const entries = [];
  for (const file of files) {
    const buffer = await readFile(file.absolutePath);
    const dimensions = readPngDimensions(buffer);
    const relativePath = file.relativePath.replaceAll("\\", "/");
    const parts = relativePath.split("/");
    entries.push({
      tileUuid: tileUuidFromRelativePath(relativePath),
      relativePath,
      category: parts.length > 1 ? parts[0] : "root",
      width: dimensions.width,
      height: dimensions.height,
      bytes: buffer.length,
      sha256: sha256(buffer),
    });
  }

  const sizes = new Map();
  for (const entry of entries) {
    const key = `${entry.width}x${entry.height}`;
    sizes.set(key, (sizes.get(key) || 0) + 1);
  }
  const uniqueTileIds = new Set(entries.map((entry) => entry.tileUuid)).size;

  return {
    schemaVersion: TILE_ATLAS_SCHEMA_VERSION,
    kind: "scrapmap.tile-atlas",
    source: TILE_ATLAS_SOURCE,
    contentFingerprint: atlasFingerprint(entries),
    previewSizes: Object.fromEntries(
      [...sizes.entries()].sort(([left], [right]) => left.localeCompare(right)),
    ),
    summary: {
      fileCount: entries.length,
      uniqueTileIds,
      variantCount: entries.length - uniqueTileIds,
      totalBytes: entries.reduce((total, entry) => total + entry.bytes, 0),
      categoryCount: new Set(entries.map((entry) => entry.category)).size,
    },
    entries,
  };
}

export async function copyTileAtlas(manifest, gameRoot, destination) {
  const sourceRoot = path.resolve(gameRoot, ...TILE_ATLAS_SOURCE.split("/"));
  const destinationRoot = path.resolve(destination);
  await mkdir(destinationRoot, { recursive: true });

  for (const entry of manifest.entries) {
    const target = path.join(destinationRoot, ...entry.relativePath.split("/"));
    await mkdir(path.dirname(target), { recursive: true });
    await copyFile(path.join(sourceRoot, ...entry.relativePath.split("/")), target);
  }
}

export async function writeManifest(manifest, outputPath) {
  const target = path.resolve(outputPath);
  await mkdir(path.dirname(target), { recursive: true });
  await writeFile(target, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
}

function usage() {
  return [
    "Usage: node tools/tile-atlas/index.mjs --game-root <Scrap Mechanic>",
    "       [--output <manifest.json>] [--copy-to <runtime tile directory>]",
    "",
    "The manifest contains relative paths and hashes only; game PNGs are not committed.",
  ].join("\n");
}

async function main() {
  const options = parseArguments(process.argv.slice(2));
  if (options.help) {
    process.stdout.write(`${usage()}\n`);
    return;
  }

  const manifest = await scanTileAtlas(options.gameRoot);
  if (options.output) {
    await writeManifest(manifest, options.output);
  }
  if (options.copyTo) {
    await copyTileAtlas(manifest, options.gameRoot, options.copyTo);
  }

  process.stdout.write(
    `${JSON.stringify(
      {
        contentFingerprint: manifest.contentFingerprint,
        ...manifest.summary,
        output: options.output ? path.resolve(options.output) : null,
        copiedTo: options.copyTo ? path.resolve(options.copyTo) : null,
      },
      null,
      2,
    )}\n`,
  );
}

if (
  process.argv[1] &&
  path.resolve(fileURLToPath(import.meta.url)) === path.resolve(process.argv[1])
) {
  main().catch((error) => {
    process.stderr.write(`tile-atlas: ${error.message}\n`);
    process.exitCode = 1;
  });
}
