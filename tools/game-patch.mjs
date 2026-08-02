#!/usr/bin/env node
// Applies and reverts the ScrapMap game-script patch.
//
// Scrap Mechanic validates Lua script checksums when joining a server: the host
// sends m_serverGameInfo.m_vecFileChecksums and the client compares its own
// files, refusing to connect on any mismatch. So the patched scripts are only
// usable when every player in the session has byte-identical files.
//
//   status   what is installed, with hashes both players can compare
//   snapshot record the current (vanilla) files as the restore baseline
//   apply    install the addon files and the injected blocks
//   revert   restore the vanilla files and remove the addon files
//
// Run `revert` before joining a server whose host is unpatched.

import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const PATCH_ROOT = join(REPO, "game-patch");
const VANILLA_ROOT = join(PATCH_ROOT, "vanilla");

const MARKER_BEGIN = "-- SCRAPMAP ADDON BEGIN";
const MARKER_END = "-- SCRAPMAP ADDON END";
// The pre-existing telemetry patch used its own marker pair.
const LEGACY_MARKERS = [["-- SCRAPMAP TELEMETRY ADDON BEGIN", "-- SCRAPMAP TELEMETRY ADDON END"]];

/** Files copied in wholesale; they do not exist in a vanilla install. */
const ADDON_FILES = [
  "Survival/Scripts/terrain/ScrapMapAtlasBake.lua",
  "Survival/Scripts/terrain/ScrapMapLayoutExport.lua",
  "Survival/Scripts/game/ScrapMapTelemetry.lua",
  "Survival/Scripts/game/ScrapMapPoiShoot.lua",
];

/**
 * Blocks appended to stock files. Appending (rather than editing in place)
 * keeps the patch exactly reversible and survives game updates that change the
 * surrounding code.
 */
const INJECTIONS = [
  {
    file: "Survival/Scripts/game/SurvivalGame.lua",
    body: [
      `dofile( "$SURVIVAL_DATA/Scripts/game/ScrapMapTelemetry.lua" )`,
      `dofile( "$SURVIVAL_DATA/Scripts/game/ScrapMapPoiShoot.lua" )`,
    ].join("\n"),
  },
  {
    file: "Survival/Scripts/terrain/terrain_overworld.lua",
    // Load() is defined above, so wrap it rather than editing its body.
    body: [
      `dofile( "$SURVIVAL_DATA/Scripts/terrain/ScrapMapLayoutExport.lua" )`,
      `dofile( "$SURVIVAL_DATA/Scripts/terrain/ScrapMapAtlasBake.lua" )`,
      ``,
      `local __scrapmapInnerLoad = Load`,
      `function Load()`,
      `\tlocal loaded = __scrapmapInnerLoad()`,
      `\tif loaded then`,
      `\t\tScrapMapExportLayout()`,
      `\t\tScrapMapBakeAtlas()`,
      `\tend`,
      `\treturn loaded`,
      `end`,
    ].join("\n"),
  },
];

const MANAGED = [...ADDON_FILES, ...INJECTIONS.map((entry) => entry.file)];

const GAME_DIRECTORY = join("steamapps", "common", "Scrap Mechanic");

const isGameRoot = (root) => existsSync(join(root, "Survival", "Scripts"));

/** Looks for the install in the usual Steam library layouts on every drive. */
function discoverGameRoot() {
  const libraries = ["SteamLibrary", join("Program Files (x86)", "Steam"), "Steam"];
  for (const letter of "CDEFGHIJKLMNOPQRSTUVWXYZ") {
    for (const library of libraries) {
      const candidate = join(`${letter}:${sep}`, library, GAME_DIRECTORY);
      if (isGameRoot(candidate)) return candidate;
    }
  }
  return null;
}

function gameRoot() {
  const flag = process.argv.find((value) => value.startsWith("--game="));
  const explicit = flag ? flag.slice("--game=".length) : process.env.SCRAPMAP_GAME_ROOT;
  if (explicit) {
    if (!isGameRoot(explicit)) {
      throw new Error(`Not a Scrap Mechanic install: ${explicit}`);
    }
    return explicit;
  }
  const found = discoverGameRoot();
  if (!found) {
    throw new Error(
      "Could not find Scrap Mechanic. Pass --game=<path> or set SCRAPMAP_GAME_ROOT.",
    );
  }
  return found;
}

/** Collapses CRLF and lone CR to LF so installed files are byte-stable. */
const normaliseEndings = (text) => text.replace(/\r\n?/g, "\n");

const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");

function hashFile(path) {
  return existsSync(path) ? sha256(readFileSync(path)) : null;
}

/** Removes every ScrapMap block, current or legacy, from a file's text. */
function stripBlocks(text) {
  let out = text;
  for (const [begin, end] of [[MARKER_BEGIN, MARKER_END], ...LEGACY_MARKERS]) {
    for (;;) {
      const start = out.indexOf(begin);
      if (start === -1) break;
      const stop = out.indexOf(end, start);
      if (stop === -1) {
        out = out.slice(0, start);
        break;
      }
      out = out.slice(0, start) + out.slice(stop + end.length);
    }
  }
  return out.replace(/\s+$/, "\n");
}

const isPatched = (text) =>
  text.includes(MARKER_BEGIN) || LEGACY_MARKERS.some(([begin]) => text.includes(begin));

function commandSnapshot(root) {
  const dirty = MANAGED.filter((relative) => {
    const path = join(root, relative);
    return existsSync(path) && isPatched(readFileSync(path, "utf8"));
  });
  if (dirty.length) {
    console.error("Refusing to snapshot: these files still carry ScrapMap blocks.\n");
    for (const file of dirty) console.error(`  ${file}`);
    console.error(
      "\nRun `revert` first, or restore a clean install with Steam ->" +
        " Properties -> Installed Files -> Verify integrity of game files.",
    );
    process.exitCode = 1;
    return;
  }

  for (const relative of INJECTIONS.map((entry) => entry.file)) {
    const source = join(root, relative);
    if (!existsSync(source)) {
      console.error(`missing ${relative}`);
      process.exitCode = 1;
      return;
    }
    const target = join(VANILLA_ROOT, relative);
    mkdirSync(dirname(target), { recursive: true });
    copyFileSync(source, target);
    console.log(`snapshot  ${relative}  ${hashFile(target).slice(0, 16)}`);
  }
  console.log("\nVanilla baseline recorded. `apply` and `revert` are now exact.");
}

function commandApply(root) {
  for (const relative of INJECTIONS.map((entry) => entry.file)) {
    if (!existsSync(join(VANILLA_ROOT, relative))) {
      console.error(
        `No vanilla baseline for ${relative}.\n` +
          "Restore a clean install (Steam -> Verify integrity of game files), then run `snapshot`.",
      );
      process.exitCode = 1;
      return;
    }
  }

  for (const relative of ADDON_FILES) {
    const source = join(PATCH_ROOT, relative);
    if (!existsSync(source)) {
      console.log(`skip      ${relative} (not in repo)`);
      continue;
    }
    const target = join(root, relative);
    mkdirSync(dirname(target), { recursive: true });
    // Install with normalised line endings rather than copying byte-for-byte.
    // A checkout on a machine with core.autocrlf can rewrite these files, and
    // the host/client checksum comparison fails on a single changed byte.
    writeFileSync(target, normaliseEndings(readFileSync(source, "utf8")));
    console.log(`addon     ${relative}`);
  }

  for (const { file, body } of INJECTIONS) {
    // Always rebuild from the vanilla baseline so apply is idempotent.
    const base = readFileSync(join(VANILLA_ROOT, file), "utf8");
    const text = `${stripBlocks(base)}\n${MARKER_BEGIN}\n${body}\n${MARKER_END}\n`;
    writeFileSync(join(root, file), text);
    console.log(`inject    ${file}`);
  }

  console.log("\nPatched. Both players must run identical files to share a server:");
  commandStatus(root, true);
}

function commandRevert(root) {
  for (const relative of ADDON_FILES) {
    const target = join(root, relative);
    if (existsSync(target)) {
      rmSync(target);
      console.log(`remove    ${relative}`);
    }
  }

  for (const { file } of INJECTIONS) {
    const target = join(root, file);
    const baseline = join(VANILLA_ROOT, file);
    if (existsSync(baseline)) {
      copyFileSync(baseline, target);
      console.log(`restore   ${file}`);
    } else if (existsSync(target)) {
      // No baseline: fall back to stripping our blocks in place.
      const stripped = stripBlocks(readFileSync(target, "utf8"));
      writeFileSync(target, stripped);
      console.log(`strip     ${file} (no baseline; verify game files if joining fails)`);
    }
  }

  console.log("\nReverted. Multiplayer with unpatched hosts should work again.");
}

function commandStatus(root, brief = false) {
  if (!brief) {
    console.log(`game: ${root}\n`);
  }
  for (const relative of MANAGED) {
    const path = join(root, relative);
    const hash = hashFile(path);
    const state = !hash
      ? "absent"
      : ADDON_FILES.includes(relative)
        ? "addon"
        : isPatched(readFileSync(path, "utf8"))
          ? "patched"
          : "vanilla";
    console.log(`  ${state.padEnd(8)} ${hash ? hash.slice(0, 16) : "-".repeat(16)}  ${relative}`);
  }
  if (!brief) {
    const baseline = existsSync(VANILLA_ROOT) ? readdirSync(VANILLA_ROOT).length : 0;
    console.log(
      `\nvanilla baseline: ${baseline ? "recorded" : "MISSING - run `snapshot` on a clean install"}`,
    );
  }
}

const command = process.argv[2];
try {
  const root = gameRoot();
  if (command === "snapshot") commandSnapshot(root);
  else if (command === "apply") commandApply(root);
  else if (command === "revert") commandRevert(root);
  else if (command === "status" || !command) commandStatus(root);
  else {
    console.error(`unknown command: ${command}`);
    console.error("usage: node tools/game-patch.mjs [status|snapshot|apply|revert] [--game=PATH]");
    process.exitCode = 1;
  }
} catch (error) {
  console.error(String(error.message || error));
  process.exitCode = 1;
}
