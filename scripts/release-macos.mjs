#!/usr/bin/env node
/**
 * Builds, signs, notarises and uploads the macOS release from this machine.
 *
 * The signing key stays in the login keychain instead of being exported to a
 * CI secret, which is why `release.yml` builds Windows and Linux but not this.
 * The Apple ID and its app-specific password are read from the keychain too,
 * so nothing has to be kept in a shell profile to make a release:
 *
 *   security add-generic-password -s nethack-tiles-notary -a you@example.com -w
 *
 * (`-w` with no value prompts, keeping the password out of shell history.)
 * Run this after `npm run release` has tagged the version and the tag has been
 * pushed, so there is a draft release to attach to:
 *
 *   npm run release:macos
 *   npm run release:macos -- --skip-build   # re-verify and upload what is built
 *   npm run release:macos -- --no-upload    # build and verify, attach nothing
 */

import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = dirname(dirname(fileURLToPath(import.meta.url)));

/**
 * The Apple developer team to sign as.
 *
 * Not a secret: it is embedded in the signature of every build we ship, and
 * naming it here means one less thing to have set correctly at release time.
 */
const TEAM_ID = "TA59XVWN77";

/** The keychain item holding the Apple ID and its app-specific password. */
const NOTARY_ITEM = "nethack-tiles-notary";

/** Where a universal build leaves its bundles. */
const BUNDLE = "src-tauri/target/universal-apple-darwin/release/bundle";

const APP = "NetHack Tiles Client.app";

/** Both halves of the universal binary. */
const TARGETS = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

/**
 * Reports which Rust targets `rustup` is missing.
 *
 * A default toolchain has only the host architecture, and the bundler does not
 * notice the other one is absent until it has finished compiling the first —
 * over a minute of work before the error appears.
 *
 * @param {string} installed output of `rustup target list --installed`
 * @returns {string[]}
 */
export function missingTargets(installed) {
  const have = new Set(installed.split("\n").map((line) => line.trim().split(" ")[0]));
  return TARGETS.filter((target) => !have.has(target));
}

/**
 * Finds the Developer ID certificate for a team in `security find-identity`.
 *
 * Only this kind can be notarised. An Apple Development certificate is the one
 * most likely to be installed already — Xcode makes it unprompted — and it
 * signs a build that Apple then refuses, so it is not accepted as a fallback.
 *
 * @param {string} output
 * @param {string} teamId
 * @returns {string | null}
 */
export function pickSigningIdentity(output, teamId) {
  const pattern = new RegExp(`"(Developer ID Application: [^"]*\\(${teamId}\\))"`);
  return output.match(pattern)?.[1] ?? null;
}

/**
 * Reads the account (the Apple ID) out of `security find-generic-password`.
 *
 * @param {string} output
 * @returns {string | null}
 */
export function keychainAccount(output) {
  return output.match(/"acct"<blob>="([^"]*)"/)?.[1] ?? null;
}

/**
 * Picks this version's universal disk image out of the bundle directory.
 *
 * Nothing clears that directory between builds, so matching on the version is
 * what keeps a stale image from being uploaded under a new tag.
 *
 * @param {string[]} files
 * @param {string} version
 * @returns {string | null}
 */
export function pickLocalDmg(files, version) {
  const wanted = `_${version}_universal.dmg`;
  return files.find((f) => f.endsWith(wanted) && !f.startsWith("rw.")) ?? null;
}

/**
 * Reads `spctl --assess` output as Gatekeeper would.
 *
 * Signing and notarisation are separate steps and the second can fail on its
 * own, leaving a build that is signed, looks finished, and is still refused on
 * every machine but this one.
 *
 * @param {string} output
 * @returns {boolean}
 */
export function isNotarized(output) {
  return /^source=Notarized Developer ID$/m.test(output) && /: accepted$/m.test(output);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  try {
    main();
  } catch (e) {
    fail(e instanceof Error ? e.message : String(e));
  }
}

function main() {
  if (process.platform !== "darwin") {
    fail("the macOS release has to be built on macOS");
  }

  const argv = process.argv.slice(2);
  const skipBuild = argv.includes("--skip-build");
  const upload = !argv.includes("--no-upload");

  const version = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8")).version;
  const tag = `v${version}`;

  if (!skipBuild) {
    requireTargets();
    const credentials = resolveCredentials();
    console.log(`building ${tag} as ${credentials.APPLE_SIGNING_IDENTITY}`);
    // Notarisation is a round trip to Apple and the wait is most of the time
    // this takes; the output is left visible so it does not look hung.
    run("npx", ["tauri", "build", "--target", "universal-apple-darwin"], credentials);
  }

  const dmgDir = join(ROOT, BUNDLE, "dmg");
  if (!existsSync(dmgDir)) {
    fail(`no build to release: ${BUNDLE}/dmg does not exist`);
  }
  const dmg = pickLocalDmg(readdirSync(dmgDir), version);
  if (!dmg) {
    fail(`no _${version}_universal.dmg in ${BUNDLE}/dmg; build without --skip-build`);
  }

  verify(join(ROOT, BUNDLE, "macos", APP));
  const path = join(dmgDir, dmg);
  console.log(`  ${dmg}`);

  if (!upload) {
    console.log(`built and verified, not uploaded:\n  ${path}`);
    return;
  }

  requireDraft(tag);
  run("gh", ["release", "upload", tag, path, "--clobber"]);
  console.log(
    `attached to the ${tag} draft.\n` +
      "Publishing that draft is what updates the Homebrew tap.",
  );
}

/**
 * Checks for both architectures before anything is compiled, since the
 * bundler builds one and only then discovers the other is unavailable.
 */
function requireTargets() {
  const missing = missingTargets(capture("rustup", ["target", "list", "--installed"]));
  if (missing.length) {
    fail(`rustup is missing ${missing.join(" and ")}:\n  rustup target add ${missing.join(" ")}`);
  }
}

/**
 * Collects what the bundler needs to sign and notarise, from the keychain
 * unless the environment already says otherwise.
 *
 * @returns {Record<string, string>}
 */
function resolveCredentials() {
  const teamId = process.env.APPLE_TEAM_ID?.trim() || TEAM_ID;

  const identity =
    process.env.APPLE_SIGNING_IDENTITY?.trim() ||
    pickSigningIdentity(capture("security", ["find-identity", "-v", "-p", "codesigning"]), teamId);
  if (!identity) {
    fail(
      `no "Developer ID Application: ... (${teamId})" certificate in the keychain.\n` +
        "  create one at https://developer.apple.com/account/resources/certificates/add\n" +
        "  and double-click the download to install it",
    );
  }

  let stored = "";
  try {
    stored = capture("security", ["find-generic-password", "-s", NOTARY_ITEM]);
  } catch {
    fail(
      `no ${NOTARY_ITEM} item in the keychain. Store the app-specific password from\n` +
        "  https://appleid.apple.com (Sign-In and Security > App-Specific Passwords):\n" +
        `    security add-generic-password -s ${NOTARY_ITEM} -a you@example.com -w`,
    );
  }

  const account = process.env.APPLE_ID?.trim() || keychainAccount(stored);
  if (!account) {
    fail(`the ${NOTARY_ITEM} keychain item has no account; re-add it with -a you@example.com`);
  }

  const password =
    process.env.APPLE_PASSWORD?.trim() ||
    capture("security", ["find-generic-password", "-s", NOTARY_ITEM, "-w"]).trim();

  return {
    APPLE_SIGNING_IDENTITY: identity,
    APPLE_ID: account,
    APPLE_PASSWORD: password,
    APPLE_TEAM_ID: teamId,
  };
}

/**
 * Checks the built app the way a stranger's Mac will.
 *
 * @param {string} app
 */
function verify(app) {
  if (!existsSync(app)) {
    fail(`the build produced no ${APP}`);
  }

  // --assess reads the signature and the notarisation ticket; -t install is
  // the policy used for something that arrived as a download.
  const assessed = capture("spctl", ["--assess", "-vvv", "-t", "install", app], true);
  if (!isNotarized(assessed)) {
    fail(
      `Gatekeeper would refuse this build:\n${indent(assessed)}\n` +
        "  a rejected build means notarisation did not finish, not that the upload failed",
    );
  }

  // A ticket that is only on Apple's servers still leaves a first launch
  // needing the network; stapling puts it inside the bundle.
  capture("xcrun", ["stapler", "validate", app]);
  console.log("notarised and stapled");
}

/**
 * A draft release is made by the tag's CI run, and uploading to a tag that has
 * none creates a *published* release instead — announcing the build early and
 * with the other platforms missing.
 *
 * @param {string} tag
 */
function requireDraft(tag) {
  try {
    capture("gh", ["release", "view", tag]);
  } catch {
    fail(
      `there is no ${tag} release to attach to.\n` +
        `  push the tag first (git push origin main ${tag}) and let the workflow draft it`,
    );
  }
}

/**
 * @param {string} file
 * @param {string[]} args
 * @param {Record<string, string>} [env] added to this process's environment
 */
function run(file, args, env) {
  execFileSync(file, args, { cwd: ROOT, stdio: "inherit", env: { ...process.env, ...env } });
}

/**
 * Runs a tool and returns everything it said.
 *
 * Both streams are collected because these tools disagree about which to use:
 * `spctl` writes its verdict to stderr even when it accepts, while `security`
 * puts the attributes and the password on stdout.
 *
 * @param {string} file
 * @param {string[]} args
 * @param {boolean} [allowFailure] tools that report their verdict by exit code
 * @returns {string}
 */
function capture(file, args, allowFailure = false) {
  const result = spawnSync(file, args, { cwd: ROOT, encoding: "utf8" });
  if (result.error) throw result.error;
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`.trim();
  if (result.status !== 0 && !allowFailure) {
    throw new Error(output || `${file} exited ${result.status}`);
  }
  return output;
}

/** @param {string} text */
function indent(text) {
  return text
    .split("\n")
    .map((line) => `    ${line}`)
    .join("\n");
}

/** @param {string} message */
function fail(message) {
  console.error(`release:macos: ${message}`);
  process.exit(1);
}
