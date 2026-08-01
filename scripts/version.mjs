/**
 * The version number lives in five files that have to agree, so the rewriting
 * of each one is a pure function with a test rather than a sed line in a
 * release script nobody can run twice.
 *
 * @see release.mjs for the command that uses these.
 */

/** X.Y.Z and nothing else: no `v` prefix, no `-beta.1` suffix. */
const SEMVER = /^(\d+)\.(\d+)\.(\d+)$/;

/**
 * Works out the version a release is going to.
 *
 * `bump` is `major`, `minor`, `patch`, or the exact version to release.
 *
 * @param {string} current
 * @param {string} bump
 * @returns {string}
 */
export function nextVersion(current, bump) {
  const parts = SEMVER.exec(current);
  if (!parts) throw new Error(`the current version is not X.Y.Z: ${current}`);
  const [major, minor, patch] = parts.slice(1).map(Number);

  switch (bump) {
    case "major":
      return `${major + 1}.0.0`;
    case "minor":
      return `${major}.${minor + 1}.0`;
    case "patch":
      return `${major}.${minor}.${patch + 1}`;
    default:
      // Anything else is meant to be a literal version. Prereleases and `v`
      // prefixes are refused here rather than breaking the Homebrew cask,
      // which builds its download URL from a bare X.Y.Z.
      if (!SEMVER.test(bump)) {
        throw new Error(`not a bump keyword and not an X.Y.Z version: ${bump}`);
      }
      return bump;
  }
}

/**
 * Rewrites the version in a package.json or package-lock.json.
 *
 * npm repeats the package's own version inside `packages[""]` of the lockfile,
 * and leaving that stale makes `npm ci` rewrite the file mid-build.
 *
 * @param {string} text
 * @param {string} version
 * @returns {string}
 */
export function withJsonVersion(text, version) {
  const doc = JSON.parse(text);
  doc.version = version;
  if (doc.packages?.[""]) doc.packages[""].version = version;
  return `${JSON.stringify(doc, null, 2)}\n`;
}

/**
 * Rewrites the `version` of the `[package]` table in a Cargo manifest.
 *
 * Scoped to that one table so the dependency versions below it survive.
 *
 * @param {string} text
 * @param {string} version
 * @returns {string}
 */
export function withCargoVersion(text, version) {
  const [start, end] = tableRange(text, "[package]");
  const table = text.slice(start, end);
  const rewritten = table.replace(/^version = "[^"]*"$/m, `version = "${version}"`);
  if (rewritten === table) {
    throw new Error("no version to rewrite in the [package] table");
  }
  return text.slice(0, start) + rewritten + text.slice(end);
}

/**
 * Rewrites one package's version in a Cargo.lock.
 *
 * The lockfile records our own crate alongside every dependency, and cargo
 * rewrites the file on the next build if the two disagree — which would leave
 * a release commit that is dirty the moment it is checked out.
 *
 * @param {string} text
 * @param {string} name
 * @param {string} version
 * @returns {string}
 */
export function withLockVersion(text, name, version) {
  const entry = new RegExp(`(name = "${escapeRegExp(name)}"\\nversion = )"[^"]*"`);
  if (!entry.test(text)) {
    throw new Error(`no [[package]] entry named ${name}`);
  }
  return text.replace(entry, `$1"${version}"`);
}

/**
 * Locates a TOML table's body: from its header to the next header, or the end.
 *
 * @param {string} text
 * @param {string} header
 * @returns {[number, number]}
 */
function tableRange(text, header) {
  const start = text.indexOf(header);
  if (start === -1) throw new Error(`no ${header} table`);
  const next = text.indexOf("\n[", start + header.length);
  return [start, next === -1 ? text.length : next];
}

/** @param {string} s */
function escapeRegExp(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
