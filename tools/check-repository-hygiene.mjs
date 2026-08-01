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
