import { describe, expect, it } from "vitest";
import { metaByte } from "./keys";

/** A keydown as the webview reports it, with Option held unless overridden. */
function press(over: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return {
    altKey: true,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
    code: "KeyL",
    key: "¬",
    ...over,
  } as KeyboardEvent;
}

describe("metaByte", () => {
  it("turns Option+l into the byte NetHack reads as M-l", () => {
    expect(metaByte(press({ code: "KeyL" }))).toBe(0xec);
  });

  it("reads the physical key rather than the character macOS composes", () => {
    // Option+l produces "¬" on a US layout. Going by `key` would send a
    // negation sign and NetHack would see no command at all.
    expect(metaByte(press({ code: "KeyN", key: "˜" }))).toBe(0x80 | 0x6e);
  });

  it("sends the capital when Shift is held", () => {
    expect(metaByte(press({ code: "KeyN", shiftKey: true }))).toBe(0x80 | 0x4e);
  });

  it("handles digits", () => {
    expect(metaByte(press({ code: "Digit2", key: "™" }))).toBe(0x80 | 0x32);
  });

  it("ignores a key pressed without Option", () => {
    expect(metaByte(press({ altKey: false, key: "l" }))).toBeNull();
  });

  it("ignores Command chords, which belong to the OS", () => {
    expect(metaByte(press({ metaKey: true }))).toBeNull();
  });

  it("ignores Control chords, which xterm already encodes", () => {
    expect(metaByte(press({ ctrlKey: true }))).toBeNull();
  });

  it("ignores keys with no meta form", () => {
    expect(metaByte(press({ code: "ArrowLeft", key: "ArrowLeft" }))).toBeNull();
    expect(metaByte(press({ code: "F5", key: "F5" }))).toBeNull();
  });
});
