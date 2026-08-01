import assert from "node:assert/strict";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import {
  copyTileAtlas,
  readPngDimensions,
  scanTileAtlas,
  writeManifest,
} from "../tools/tile-atlas/index.mjs";

const ONE_BY_ONE_PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
  "base64",
);

test("tile atlas scanner validates PNGs and keeps duplicate UUID variants", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "scrapmap-tile-atlas-"));
  try {
    const tiles = path.join(root, "Survival", "Terrain", "Tiles");
    const first = "11111111-1111-4111-8111-111111111111.png";
    const second = "22222222-2222-4222-8222-222222222222.png";
    await mkdir(path.join(tiles, "meadow"), { recursive: true });
    await writeFile(path.join(tiles, first), ONE_BY_ONE_PNG);
    await writeFile(path.join(tiles, "meadow", first), ONE_BY_ONE_PNG);
    await writeFile(path.join(tiles, "meadow", second), ONE_BY_ONE_PNG);

    assert.deepEqual(readPngDimensions(ONE_BY_ONE_PNG), {
      width: 1,
      height: 1,
    });

    const manifest = await scanTileAtlas(root);
    assert.equal(manifest.summary.fileCount, 3);
    assert.equal(manifest.summary.uniqueTileIds, 2);
    assert.equal(manifest.summary.variantCount, 1);
    assert.deepEqual(manifest.previewSizes, { "1x1": 3 });
    assert.equal(
      manifest.entries.filter(
        (entry) => entry.tileUuid === "11111111-1111-4111-8111-111111111111",
      ).length,
      2,
    );
    assert.ok(
      manifest.entries.every(
        (entry) => !path.isAbsolute(entry.relativePath),
      ),
    );

    const output = path.join(root, "manifest.json");
    const copied = path.join(root, "copied");
    await writeManifest(manifest, output);
    await copyTileAtlas(manifest, root, copied);
    assert.equal(JSON.parse(await readFile(output, "utf8")).contentFingerprint, manifest.contentFingerprint);
    assert.deepEqual(
      await readFile(path.join(copied, "meadow", second)),
      ONE_BY_ONE_PNG,
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("tile atlas scanner rejects malformed PNG data", () => {
  assert.throws(() => readPngDimensions(Buffer.from("not-a-png")), /Invalid PNG/);
});
