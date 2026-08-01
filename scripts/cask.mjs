/**
 * Builds the Homebrew cask that `brew install statico/tap/nethack-tiles-client`
 * reads. Run from .github/workflows/tap.yml when a release is published, so a
 * released version and the tap never drift apart by hand.
 */

const REPO = "https://github.com/statico/nethack-tiles-client";

/** What the .app is called once the disk image is mounted. */
const APP = "NetHack Tiles Client.app";

/** Matches how ProjectDirs and the bundle identifier name our state. */
const IDENTIFIER = "com.ian.nethack-tiles";

/**
 * Picks the macOS build out of a release's assets.
 *
 * @param {Array<{name: string}>} assets
 * @returns {{name: string}}
 */
export function pickDmg(assets) {
  const dmg = assets.find((a) => a.name.endsWith("_universal.dmg"));
  if (!dmg) throw new Error("the release has no *_universal.dmg to point the cask at");
  return dmg;
}

/**
 * Turns a released asset name into one the cask can rebuild for any version.
 *
 * GitHub renames assets on upload (spaces become dots), so the shape of the
 * filename is discovered from a real release rather than assumed here.
 *
 * @param {string} asset
 * @param {string} version
 * @returns {string}
 */
export function urlTemplate(asset, version) {
  if (!asset.includes(version)) {
    throw new Error(`asset name ${asset} does not contain the version ${version}`);
  }
  return asset.replaceAll(version, "#{version}");
}

/**
 * @param {{version: string, sha256: string, asset: string}} release
 * @returns {string}
 */
export function renderCask({ version, sha256, asset }) {
  const file = urlTemplate(asset, version);
  return `cask "nethack-tiles-client" do
  version "${version}"
  sha256 "${sha256}"

  url "${REPO}/releases/download/v#{version}/${file}"
  name "NetHack Tiles Client"
  desc "Play NetHack on the public servers with graphical tiles"
  homepage "${REPO}"

  livecheck do
    url :url
    strategy :github_latest
  end

  depends_on macos: ">= :big_sur"

  app "${APP}"

  zap trash: [
    "~/Library/Application Support/${IDENTIFIER}",
    "~/Library/Saved Application State/${IDENTIFIER}.savedState",
  ]

  caveats do
    <<~EOS
      Saved server passwords live in the login keychain, not in a file, so they
      survive an uninstall and are not removed by \`brew uninstall --zap\`.
    EOS
  end
end
`;
}
