#!/usr/bin/env node
/**
 * Writes the Homebrew cask for a released version.
 *
 *   node scripts/update-tap.mjs \
 *     --version 0.1.1 \
 *     --asset NetHack.Tiles.Client_0.1.1_universal.dmg \
 *     --sha256 <64 hex chars> \
 *     --out ../tap/Casks/nethack-tiles-client.rb
 *
 * Called by .github/workflows/tap.yml, which has already downloaded the disk
 * image and checksummed it.
 */

import { mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

import { renderCask } from "./cask.mjs";

try {
  const args = parseArgs(process.argv.slice(2));
  const cask = renderCask({
    version: args.version,
    sha256: args.sha256,
    asset: args.asset,
  });
  mkdirSync(dirname(args.out), { recursive: true });
  writeFileSync(args.out, cask);
  console.log(`wrote ${args.out} for ${args.version}`);
} catch (e) {
  console.error(`update-tap: ${e instanceof Error ? e.message : e}`);
  process.exit(1);
}

/**
 * @param {string[]} argv
 * @returns {{version: string, asset: string, sha256: string, out: string}}
 */
function parseArgs(argv) {
  /** @type {Record<string, string>} */
  const args = {};
  for (let i = 0; i < argv.length; i += 2) {
    if (!argv[i].startsWith("--")) throw new Error(`unexpected argument ${argv[i]}`);
    args[argv[i].slice(2)] = argv[i + 1];
  }

  for (const key of ["version", "asset", "sha256", "out"]) {
    if (!args[key]) throw new Error(`--${key} is required`);
  }
  // A truncated checksum would install anything; Homebrew only compares.
  if (!/^[0-9a-f]{64}$/.test(args.sha256)) {
    throw new Error(`--sha256 is not a sha256 digest: ${args.sha256}`);
  }
  return /** @type {any} */ (args);
}
