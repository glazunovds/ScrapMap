import fs from "node:fs/promises";
import path from "node:path";
import process from "node:process";

const root = path.resolve(import.meta.dirname, "..");
const ignoredDirectories = new Set([
  ".git",
  "dist",
  "node_modules",
  "runtime",
  "target",
]);
const ignoredFiles = new Set([
  "tools/check-repository-hygiene.mjs",
]);
const textExtensions = new Set([
  ".css",
  ".html",
  ".js",
  ".json",
  ".md",
  ".mjs",
  ".ps1",
  ".rs",
  ".toml",
  ".ts",
  ".yml",
  ".yaml",
]);
const blockedPatterns = [
  {
    label: "local Windows user or workspace path",
    expression:
      /[A-Za-z]:[\\/](?:Users[\\/][^\\/\r\n]+|SteamLibrary|work[\\/]ideas)[\\/]/i,
  },
  {
    label: "temporary Codex clipboard capture",
    expression: /codex-clipboard-[a-z0-9-]+\.(?:jpe?g|png|webp)/i,
  },
  {
    label: "private key material",
    expression: /-----BEGIN (?:EC |OPENSSH |RSA )?PRIVATE KEY-----/,
  },
  {
    label: "literal bearer credential",
    expression: /\bBearer\s+[A-Za-z0-9._~+/-]{24,}={0,2}\b/,
  },
  {
    label: "temporary ngrok address",
    expression: /https?:\/\/[a-z0-9-]+\.ngrok-free\.app/i,
  },
];

async function collectFiles(directory) {
  const entries = await fs.readdir(directory, { withFileTypes: true });
  const files = [];

  for (const entry of entries) {
    if (entry.isDirectory() && ignoredDirectories.has(entry.name)) {
      continue;
    }

    const absolutePath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await collectFiles(absolutePath)));
    } else if (entry.isFile() && textExtensions.has(path.extname(entry.name))) {
      files.push(absolutePath);
    }
  }

  return files;
}

const violations = [];
for (const absolutePath of await collectFiles(root)) {
  const relativePath = path.relative(root, absolutePath).replaceAll("\\", "/");
  if (ignoredFiles.has(relativePath)) {
    continue;
  }

  const contents = await fs.readFile(absolutePath, "utf8");
  for (const pattern of blockedPatterns) {
    if (pattern.expression.test(contents)) {
      violations.push(`${relativePath}: ${pattern.label}`);
    }
  }
}

// The interface strings are only as good as their keys. Two failures shipped
// today that a build cannot catch: a key with no entry renders as the key, and
// a call in a scope where the helper is shadowed throws at runtime -- which
// aborted the layout and left the map permanently unresolved.
{
  const english = JSON.parse(
    await fs.readFile(new URL("../public/map/locales/en.json", import.meta.url), "utf8"),
  );
  const russian = JSON.parse(
    await fs.readFile(new URL("../public/map/locales/ru.json", import.meta.url), "utf8"),
  );

  for (const key of Object.keys(english)) {
    if (!(key in russian)) violations.push(`ru.json: no entry for ${key}`);
  }
  for (const key of Object.keys(russian)) {
    if (!(key in english)) violations.push(`en.json: no entry for ${key}, which is the fallback`);
  }

  for (const relative of ["public/map/app.js", "public/map/overlay-bridge.js"]) {
    const source = await fs.readFile(new URL(`../${relative}`, import.meta.url), "utf8");

    for (const [, key] of source.matchAll(/\btr\("([A-Z0-9_]+)"/g)) {
      if (!(key in english)) violations.push(`${relative}: tr("${key}") has no entry in en.json`);
    }

    // A local binding of the helper's name turns every call in that scope into
    // a call on something else -- a DOM node, in the case that shipped.
    if (/(?:const|let|var)\s+tr\s*=(?!\s*\(key)/.test(source)) {
      violations.push(`${relative}: something shadows the tr() string helper`);
    }
  }

  for (const relative of ["public/map/index.html"]) {
    const markup = await fs.readFile(new URL(`../${relative}`, import.meta.url), "utf8");
    for (const [, key] of markup.matchAll(/data-i18n="([A-Z0-9_]+)"/g)) {
      if (!(key in english)) violations.push(`${relative}: data-i18n="${key}" has no entry`);
    }
    for (const [, attrs] of markup.matchAll(/data-i18n-attr="([^"]+)"/g)) {
      for (const pair of attrs.split(",")) {
        const key = pair.split(":")[1]?.trim();
        if (key && !(key in english)) {
          violations.push(`${relative}: data-i18n-attr key ${key} has no entry`);
        }
      }
    }
  }
}

// A rule written against a class nothing carries is silently dead, and it
// reads exactly like a rule that works. The compact overlay hid
// `.hover-highlight` while the element was `.map-hover-cell`, so the rule
// never applied and the hover square kept painting over the minimap.
{
  const styles = ["public/map/styles.css", "public/map/overlay.css"];
  const consumers = ["public/map/index.html", "public/map/app.js", "public/map/overlay-bridge.js"];
  let carried = "";
  for (const relative of consumers) {
    carried += await fs.readFile(new URL(`../${relative}`, import.meta.url), "utf8");
  }

  for (const relative of styles) {
    const sheet = await fs.readFile(new URL(`../${relative}`, import.meta.url), "utf8");
    // Selectors only: a class name inside a declaration block would be a value.
    const selectors = sheet
      .replace(/\/\*[\s\S]*?\*\//g, "")
      .replace(/\{[^{}]*\}/g, "{}");
    const names = new Set(
      Array.from(selectors.matchAll(/\.(-?[A-Za-z_][\w-]*)/g), ([, name]) => name),
    );
    for (const name of names) {
      if (!new RegExp(`\\b${name}\\b`).test(carried)) {
        violations.push(`${relative}: .${name} matches nothing in the markup or the renderer`);
      }
    }
  }
}

if (violations.length > 0) {
  process.stderr.write(
    `Repository hygiene check failed:\n${violations
      .map((violation) => `- ${violation}`)
      .join("\n")}\n`,
  );
  process.exitCode = 1;
} else {
  process.stdout.write("Repository hygiene check passed.\n");
}
