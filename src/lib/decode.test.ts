import { describe, expect, it } from "vitest";
import { decodeStream } from "./decode";

const bytes = (...b: number[]) => new Uint8Array(b);

describe("decodeStream", () => {
  it("passes ASCII through unchanged", () => {
    expect(decodeStream(bytes(0x68, 0x69))).toBe("hi");
  });

  it("leaves escape sequences alone", () => {
    expect(decodeStream(bytes(0x1b, 0x5b, 0x31, 0x6d))).toBe("\x1b[1m");
  });

  it("decodes valid UTF-8", () => {
    expect(decodeStream(bytes(0xc3, 0xba))).toBe("ú");
  });

  it("decodes a three-byte UTF-8 sequence", () => {
    expect(decodeStream(bytes(0xe2, 0x94, 0x80))).toBe("─");
  });

  it("maps NetHack's IBMgraphics bytes through CP437", () => {
    // The bytes seen on the wire from NAO. None is valid UTF-8, so xterm.js
    // drops them without this and the map never reaches a cell.
    expect(decodeStream(bytes(0xcd))).toBe("═");
    expect(decodeStream(bytes(0xce))).toBe("╬");
    expect(decodeStream(bytes(0xfa))).toBe("·");
    expect(decodeStream(bytes(0xf0))).toBe("≡");
  });

  it("gives one character per byte for a CP437 run", () => {
    // The count is what tells the overlay how many cells a write covered, so a
    // run of wall must not come out longer or shorter than it is.
    expect(decodeStream(bytes(0xcd, 0xcd, 0xcd))).toBe("═══");
    expect(decodeStream(bytes(0xcd, 0xcd, 0xcd))).toHaveLength(3);
  });

  it("treats a lead byte with nothing valid after it as CP437", () => {
    expect(decodeStream(bytes(0xcd, 0x41))).toBe("═A");
  });

  it("treats a stray continuation byte as CP437", () => {
    expect(decodeStream(bytes(0xba))).toBe("║");
  });

  it("decodes a mixed line of walls and ASCII", () => {
    // A wall run ending in the doorway NetHack drew at the top of the room.
    expect(decodeStream(bytes(0xcd, 0xcd, 0xce, 0xcd))).toBe("══╬═");
  });
});
