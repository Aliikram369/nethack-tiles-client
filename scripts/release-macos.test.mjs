import { describe, expect, test } from "vitest";

import {
  isNotarized,
  keychainAccount,
  pickLocalDmg,
  pickSigningIdentity,
} from "./release-macos.mjs";

describe("pickSigningIdentity", () => {
  const identities = [
    '  1) AAAA1111 "Apple Development: someone@example.com (9Z9Z9Z9Z9Z)"',
    '  2) BBBB2222 "Developer ID Application: Someone (TA59XVWN77)"',
    "     2 valid identities found",
  ].join("\n");

  test("finds the Developer ID for the team", () => {
    expect(pickSigningIdentity(identities, "TA59XVWN77")).toBe(
      "Developer ID Application: Someone (TA59XVWN77)",
    );
  });

  test("ignores a development certificate", () => {
    // Apple Development signs for local debugging only. It is the certificate
    // most likely to be installed already, and notarisation rejects it.
    const onlyDev = '  1) AAAA1111 "Apple Development: someone@example.com (TA59XVWN77)"';
    expect(pickSigningIdentity(onlyDev, "TA59XVWN77")).toBeNull();
  });

  test("ignores another team's Developer ID", () => {
    expect(pickSigningIdentity(identities, "ZZ00ZZ00ZZ")).toBeNull();
  });

  test("is null when the keychain has nothing", () => {
    expect(pickSigningIdentity("     0 valid identities found", "TA59XVWN77")).toBeNull();
  });
});

describe("keychainAccount", () => {
  const found = [
    'keychain: "/Users/someone/Library/Keychains/login.keychain-db"',
    'class: "genp"',
    "attributes:",
    '    "acct"<blob>="someone@example.com"',
    '    "svce"<blob>="nethack-tiles-notary"',
  ].join("\n");

  test("reads the Apple ID stored alongside the password", () => {
    expect(keychainAccount(found)).toBe("someone@example.com");
  });

  test("is null when the item has no account", () => {
    // Adding the item without -a leaves a password nobody can attribute to an
    // Apple ID, which notarytool needs as a separate argument.
    expect(keychainAccount('class: "genp"\n    "svce"<blob>="nethack-tiles-notary"')).toBeNull();
  });
});

describe("pickLocalDmg", () => {
  const files = [
    "NetHack Tiles Client_0.1.1_universal.dmg",
    "NetHack Tiles Client_0.1.1_aarch64.dmg",
    "rw.NetHack Tiles Client_0.1.1_universal.dmg",
  ];

  test("finds the universal build for the version", () => {
    expect(pickLocalDmg(files, "0.1.1")).toBe("NetHack Tiles Client_0.1.1_universal.dmg");
  });

  test("ignores the single-architecture build", () => {
    expect(pickLocalDmg(["NetHack Tiles Client_0.1.1_aarch64.dmg"], "0.1.1")).toBeNull();
  });

  test("ignores hdiutil's half-built shadow file", () => {
    // `rw.<name>.dmg` is the writable image the bundler converts and deletes;
    // uploading one would ship a disk image with no signature attached.
    expect(pickLocalDmg(["rw.NetHack Tiles Client_0.1.1_universal.dmg"], "0.1.1")).toBeNull();
  });

  test("will not pass off an older build as this version", () => {
    // The bundle directory is not cleared between builds, so the previous
    // release's .dmg is still sitting there.
    expect(pickLocalDmg(["NetHack Tiles Client_0.1.0_universal.dmg"], "0.1.1")).toBeNull();
  });
});

describe("isNotarized", () => {
  test("accepts a stapled Developer ID build", () => {
    const output = [
      "/Volumes/x/NetHack Tiles Client.app: accepted",
      "source=Notarized Developer ID",
      "origin=Developer ID Application: Someone (TA59XVWN77)",
    ].join("\n");
    expect(isNotarized(output)).toBe(true);
  });

  test("rejects a signed build that was never notarised", () => {
    // The dangerous case: signing succeeded, notarisation quietly did not, and
    // the .dmg looks finished from here while Gatekeeper refuses it elsewhere.
    const output = [
      "/Volumes/x/NetHack Tiles Client.app: rejected",
      "source=Unnotarized Developer ID",
    ].join("\n");
    expect(isNotarized(output)).toBe(false);
  });

  test("rejects an ad-hoc signed build", () => {
    expect(isNotarized("x.app: rejected\nsource=no usable signature")).toBe(false);
  });
});
