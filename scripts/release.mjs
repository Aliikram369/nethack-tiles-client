#!/usr/bin/env node
/**
 * Cuts a release: bumps the version everywhere it is written down, commits it,
 * and tags it. Pushing the tag is what actually builds and publishes — see
 * .github/workflows/release.yml.
 *
 *   pnpm run release              # 0.1.0 -> 0.1.1, the usual dot release
 *   pnpm run release -- minor     # 0.1.1 -> 0.2.0
 *   pnpm run release -- 1.0.0     # exactly that
 *   pnpm run release -- --dry-run # say what would happen, change nothing
 *   pnpm run release -- --push    # push the commit and tag when done
 */

import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import {
  nextVersion,
  withCargoVersion,
  withJsonVersion,
  withLockVersion,
} from "./version.mjs";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));

/** The crate name, as Cargo.lock files it. */
const CRATE = "nethack-tiles-client";

try {
  main();
} catch (e) {
  // A stack trace from a version-string typo helps nobody.
  fail(e instanceof Error ? e.message : String(e));
}

function main() {
  const argv = process.argv.slice(2);
  const dryRun = argv.includes("--dry-run");
  const push = argv.includes("--push");
  const anyBranch = argv.includes("--any-branch");
  const bump = argv.find((a) => !a.startsWith("--")) ?? "patch";

  const packagePath = join(ROOT, "package.json");
  const current = JSON.parse(readFileSync(packagePath, "utf8")).version;
  const version = nextVersion(current, bump);
  const tag = `v${version}`;

  if (!dryRun) {
    requireCleanTree();
    requireReleaseBranch(anyBranch);
    requireUnusedTag(tag);
  }

  /** @type {Array<[string, (text: string) => string]>} */
  const edits = [
    // pnpm-lock.yaml is not here on purpose: it records dependencies, not
    // this package's own version, so a bump leaves it untouched.
    ["package.json", (t) => withJsonVersion(t, version)],
    ["src-tauri/tauri.conf.json", (t) => withJsonVersion(t, version)],
    ["src-tauri/Cargo.toml", (t) => withCargoVersion(t, version)],
    ["src-tauri/Cargo.lock", (t) => withLockVersion(t, CRATE, version)],
  ];

  console.log(`${current} -> ${version}`);
  for (const [file, edit] of edits) {
    const path = join(ROOT, file);
    const updated = edit(readFileSync(path, "utf8"));
    if (dryRun) {
      console.log(`  would write ${file}`);
      continue;
    }
    writeFileSync(path, updated);
    console.log(`  wrote ${file}`);
  }

  if (dryRun) {
    console.log(`would commit and tag ${tag}`);
    return;
  }

  git(["add", ...edits.map(([file]) => file)]);
  git(["commit", "-m", `Release ${tag}`]);
  git(["tag", "-a", tag, "-m", `Release ${tag}`]);
  console.log(`committed and tagged ${tag}`);

  if (push) {
    git(["push", "origin", "HEAD"]);
    git(["push", "origin", tag]);
    console.log(`pushed ${tag}; the release workflow takes it from here`);
  } else {
    console.log(`nothing is published yet. To publish:\n  git push origin HEAD ${tag}`);
  }
}

/**
 * A release has to describe a commit, and a dirty tree means the build would
 * come from something nobody can check out again.
 */
function requireCleanTree() {
  if (git(["status", "--porcelain"]).trim()) {
    fail("the working tree has uncommitted changes; commit or stash them first");
  }
}

/** @param {boolean} anyBranch */
function requireReleaseBranch(anyBranch) {
  const branch = git(["rev-parse", "--abbrev-ref", "HEAD"]).trim();
  if (branch !== "main" && !anyBranch) {
    fail(`releases come from main, not ${branch} (--any-branch to override)`);
  }
}

/** @param {string} tag */
function requireUnusedTag(tag) {
  if (git(["tag", "--list", tag]).trim()) {
    fail(`${tag} already exists`);
  }
}

/**
 * @param {string[]} args
 * @returns {string}
 */
function git(args) {
  return execFileSync("git", args, { cwd: ROOT, encoding: "utf8" });
}

/** @param {string} message */
function fail(message) {
  console.error(`release: ${message}`);
  process.exit(1);
}
