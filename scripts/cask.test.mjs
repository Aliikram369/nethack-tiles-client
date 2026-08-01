import { describe, expect, test } from "vitest";

import { pickDmg, renderCask, urlTemplate } from "./cask.mjs";

describe("pickDmg", () => {
  const assets = [
    { name: "NetHack.Tiles.Client_0.1.1_amd64.deb" },
    { name: "NetHack.Tiles.Client_0.1.1_x64_en-US.msi" },
    { name: "NetHack.Tiles.Client_0.1.1_universal.dmg" },
    { name: "nethack-tiles-client_0.1.1_amd64.AppImage" },
  ];

  test("finds the universal macOS disk image", () => {
    expect(pickDmg(assets).name).toBe("NetHack.Tiles.Client_0.1.1_universal.dmg");
  });

  test("says so when the release has no disk image", () => {
    // Better a failed tap update than a cask pointing at a URL that 404s.
    expect(() => pickDmg(assets.slice(0, 2))).toThrow(/universal.*dmg/i);
  });
});

describe("urlTemplate", () => {
  test("swaps the version out for the cask's own interpolation", () => {
    expect(urlTemplate("NetHack.Tiles.Client_0.1.1_universal.dmg", "0.1.1")).toBe(
      "NetHack.Tiles.Client_#{version}_universal.dmg",
    );
  });

  test("refuses a name the version does not appear in", () => {
    // The asset naming is GitHub's, not ours -- if it stops carrying the
    // version, a hand-written URL is the only safe answer.
    expect(() => urlTemplate("nethack.dmg", "0.1.1")).toThrow(/0\.1\.1/);
  });
});

describe("renderCask", () => {
  const cask = renderCask({
    version: "0.1.1",
    sha256: "a".repeat(64),
    asset: "NetHack.Tiles.Client_0.1.1_universal.dmg",
  });

  test("states the version and checksum Homebrew verifies against", () => {
    expect(cask).toContain('version "0.1.1"');
    expect(cask).toContain(`sha256 "${"a".repeat(64)}"`);
  });

  test("builds the download URL from the version, not from one release", () => {
    expect(cask).toContain(
      'url "https://github.com/statico/nethack-tiles-client/releases/download/' +
        'v#{version}/NetHack.Tiles.Client_#{version}_universal.dmg"',
    );
    // A literal version left in the URL is the bug this guards: the cask would
    // keep serving the old build after a bump.
    expect(cask).not.toContain("0.1.1_universal.dmg");
  });

  test("installs the app under the name the bundle actually has", () => {
    expect(cask).toContain('app "NetHack Tiles Client.app"');
  });

  test("removes the profiles and keychain-adjacent state on zap", () => {
    expect(cask).toContain("~/Library/Application Support/com.ian.nethack-tiles");
  });

  test("is a cask Ruby can parse as one block", () => {
    expect(cask.startsWith('cask "nethack-tiles-client" do\n')).toBe(true);
    expect(cask.endsWith("end\n")).toBe(true);
  });
});
