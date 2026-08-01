import { describe, expect, test } from "vitest";

import {
  nextVersion,
  withCargoVersion,
  withJsonVersion,
  withLockVersion,
} from "./version.mjs";

describe("nextVersion", () => {
  test("a dot release bumps the last number", () => {
    expect(nextVersion("0.1.0", "patch")).toBe("0.1.1");
    expect(nextVersion("0.1.9", "patch")).toBe("0.1.10");
  });

  test("a minor release resets the patch", () => {
    expect(nextVersion("0.1.7", "minor")).toBe("0.2.0");
  });

  test("a major release resets everything below it", () => {
    expect(nextVersion("0.2.3", "major")).toBe("1.0.0");
  });

  test("an explicit version is taken as given", () => {
    expect(nextVersion("0.1.0", "2.5.1")).toBe("2.5.1");
  });

  test("a version that is not three numbers is refused", () => {
    expect(() => nextVersion("0.1.0", "2.5")).toThrow(/2\.5/);
    expect(() => nextVersion("0.1.0", "v2.5.1")).toThrow(/v2\.5\.1/);
    expect(() => nextVersion("0.1", "patch")).toThrow(/0\.1/);
  });

  test("a prerelease suffix is refused rather than silently dropped", () => {
    // Tauri and Cargo both accept them, but the tap cask and the tag naming
    // here assume plain X.Y.Z. Fail loudly instead of shipping a broken cask.
    expect(() => nextVersion("0.1.0", "1.0.0-beta.1")).toThrow(/beta/);
  });
});

describe("withJsonVersion", () => {
  test("rewrites the top-level version", () => {
    const before = JSON.stringify({ name: "x", version: "0.1.0" }, null, 2) + "\n";
    expect(JSON.parse(withJsonVersion(before, "0.2.0")).version).toBe("0.2.0");
  });

  test("rewrites the self-entry npm keeps in its lockfile", () => {
    const lock =
      JSON.stringify(
        { name: "x", version: "0.1.0", packages: { "": { version: "0.1.0" } } },
        null,
        2,
      ) + "\n";
    const after = JSON.parse(withJsonVersion(lock, "0.2.0"));
    expect(after.version).toBe("0.2.0");
    expect(after.packages[""].version).toBe("0.2.0");
  });

  test("leaves dependency versions alone", () => {
    const before =
      JSON.stringify(
        { version: "0.1.0", dependencies: { react: "0.1.0" } },
        null,
        2,
      ) + "\n";
    expect(JSON.parse(withJsonVersion(before, "9.9.9")).dependencies.react).toBe(
      "0.1.0",
    );
  });

  test("ends with a newline, the way npm writes these files", () => {
    const before = JSON.stringify({ version: "0.1.0" }, null, 2) + "\n";
    expect(withJsonVersion(before, "0.2.0").endsWith("}\n")).toBe(true);
  });
});

describe("withCargoVersion", () => {
  const manifest = `[package]
name = "nethack-tiles-client"
version = "0.1.0"
edition = "2021"

[dependencies]
tauri = { version = "2", features = [] }
image = "0.25.10"

[target.'cfg(unix)'.dependencies]
libc = "0.2"
`;

  test("rewrites the package version", () => {
    expect(withCargoVersion(manifest, "0.2.0")).toContain('version = "0.2.0"');
  });

  test("leaves dependency versions alone", () => {
    const after = withCargoVersion(manifest, "0.2.0");
    expect(after).toContain('image = "0.25.10"');
    expect(after).toContain('tauri = { version = "2", features = [] }');
    expect(after).toContain('libc = "0.2"');
  });

  test("refuses a manifest with no package version to rewrite", () => {
    expect(() => withCargoVersion("[dependencies]\nlibc = \"0.2\"\n", "0.2.0")).toThrow(
      /\[package\]/,
    );
  });
});

describe("withLockVersion", () => {
  const lock = `[[package]]
name = "libc"
version = "0.2.180"

[[package]]
name = "nethack-tiles-client"
version = "0.1.0"
dependencies = [
 "base64 0.23.0",
]

[[package]]
name = "serde"
version = "1.0.230"
`;

  test("rewrites only the named package", () => {
    const after = withLockVersion(lock, "nethack-tiles-client", "0.2.0");
    expect(after).toContain('name = "nethack-tiles-client"\nversion = "0.2.0"');
    expect(after).toContain('name = "libc"\nversion = "0.2.180"');
    expect(after).toContain('name = "serde"\nversion = "1.0.230"');
  });

  test("keeps the rest of the package entry", () => {
    expect(withLockVersion(lock, "nethack-tiles-client", "0.2.0")).toContain(
      'dependencies = [\n "base64 0.23.0",\n]',
    );
  });

  test("refuses a lockfile that does not mention the package", () => {
    expect(() => withLockVersion(lock, "not-here", "0.2.0")).toThrow(/not-here/);
  });
});
