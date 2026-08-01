import { describe, expect, it, vi } from "vitest";
import { paintOverlay, type OverlayTarget } from "./overlay";
import type { GlyphFlags, TilesetManifest } from "./protocol";

const manifest: TilesetManifest = {
  id: "t",
  name: "T",
  version: "v36",
  tileWidth: 16,
  tileHeight: 16,
  columns: 40,
  tileCount: 1082,
};

const noFlags: GlyphFlags = {
  hero: false,
  corpse: false,
  invisible: false,
  detected: false,
  pet: false,
  ridden: false,
  statue: false,
  objpile: false,
  bwLava: false,
  female: false,
};

/** A canvas context that records the calls the painter makes. */
function fakeContext() {
  return {
    clearRect: vi.fn(),
    drawImage: vi.fn(),
    fillRect: vi.fn(),
    strokeRect: vi.fn(),
    fillText: vi.fn(),
    beginPath: vi.fn(),
    moveTo: vi.fn(),
    lineTo: vi.fn(),
    closePath: vi.fn(),
    fill: vi.fn(),
    save: vi.fn(),
    restore: vi.fn(),
    setTransform: vi.fn(),
    fillStyle: "",
    strokeStyle: "",
    font: "",
    textAlign: "" as CanvasTextAlign,
    textBaseline: "" as CanvasTextBaseline,
    imageSmoothingEnabled: true,
  };
}

function target(overrides: Partial<OverlayTarget> = {}): OverlayTarget {
  return {
    sheet: { width: 640, height: 448 } as CanvasImageSource,
    manifest,
    cellWidth: 10,
    cellHeight: 20,
    widthPx: 800,
    heightPx: 480,
    ...overrides,
  };
}

describe("paintOverlay", () => {
  it("clears the whole canvas before drawing", () => {
    const ctx = fakeContext();
    paintOverlay(ctx as never, target(), []);

    expect(ctx.clearRect).toHaveBeenCalledWith(0, 0, 800, 480);
  });

  it("draws nothing when there are no tiles", () => {
    const ctx = fakeContext();
    paintOverlay(ctx as never, target(), []);

    expect(ctx.drawImage).not.toHaveBeenCalled();
  });

  it("maps a tile index to its sheet rect and the cell to its pixel rect", () => {
    const ctx = fakeContext();
    paintOverlay(ctx as never, target(), [
      { row: 3, col: 5, tile: 41, flags: noFlags, ch: "d" },
    ]);

    // Tile 41 at 40 columns -> column 1, row 1 of the sheet.
    // Cell (row 3, col 5) -> x = 5*10, y = 3*20.
    expect(ctx.drawImage).toHaveBeenCalledWith(
      expect.anything(),
      16,
      16,
      16,
      16,
      50,
      60,
      10,
      20,
    );
  });

  it("draws the tile at the origin for cell (0, 0) and tile 0", () => {
    const ctx = fakeContext();
    paintOverlay(ctx as never, target(), [
      { row: 0, col: 0, tile: 0, flags: noFlags, ch: "@" },
    ]);

    expect(ctx.drawImage).toHaveBeenCalledWith(
      expect.anything(),
      0,
      0,
      16,
      16,
      0,
      0,
      10,
      20,
    );
  });

  it("draws every tile it is given", () => {
    const ctx = fakeContext();
    paintOverlay(ctx as never, target(), [
      { row: 0, col: 0, tile: 1, flags: noFlags, ch: "a" },
      { row: 1, col: 1, tile: 2, flags: noFlags, ch: "b" },
      { row: 2, col: 2, tile: 3, flags: noFlags, ch: "c" },
    ]);

    expect(ctx.drawImage).toHaveBeenCalledTimes(3);
  });

  it("draws a placeholder instead of a tile when the index is off the sheet", () => {
    // This is the tileset/server version mismatch case: it must be visible,
    // not silently blank.
    const ctx = fakeContext();
    paintOverlay(ctx as never, target(), [
      { row: 1, col: 1, tile: 99999, flags: noFlags, ch: "?" },
    ]);

    expect(ctx.drawImage).not.toHaveBeenCalled();
    expect(ctx.fillText).toHaveBeenCalledWith("?", expect.any(Number), expect.any(Number));
  });

  it("marks a pet with an extra overlay on top of its tile", () => {
    const ctx = fakeContext();
    const withPet = { ...noFlags, pet: true };
    paintOverlay(ctx as never, target(), [
      { row: 1, col: 1, tile: 10, flags: withPet, ch: "d" },
    ]);

    expect(ctx.drawImage).toHaveBeenCalledTimes(1);
    expect(ctx.fill).toHaveBeenCalled();
  });

  it("does not mark a non-pet", () => {
    const ctx = fakeContext();
    paintOverlay(ctx as never, target(), [
      { row: 1, col: 1, tile: 10, flags: noFlags, ch: "d" },
    ]);

    expect(ctx.fill).not.toHaveBeenCalled();
  });

  it("tints a detected monster", () => {
    const ctx = fakeContext();
    paintOverlay(ctx as never, target(), [
      { row: 1, col: 1, tile: 10, flags: { ...noFlags, detected: true }, ch: "d" },
    ]);

    expect(ctx.fillRect).toHaveBeenCalled();
  });

  it("keeps pixel art crisp by disabling image smoothing", () => {
    const ctx = fakeContext();
    paintOverlay(ctx as never, target(), [
      { row: 0, col: 0, tile: 1, flags: noFlags, ch: "a" },
    ]);

    expect(ctx.imageSmoothingEnabled).toBe(false);
  });

  it("skips a tile that falls outside the canvas", () => {
    const ctx = fakeContext();
    paintOverlay(ctx as never, target({ widthPx: 40, heightPx: 40 }), [
      { row: 50, col: 50, tile: 1, flags: noFlags, ch: "a" },
    ]);

    expect(ctx.drawImage).not.toHaveBeenCalled();
  });
});

describe("paintOverlay reporting", () => {
  it("reports nothing drawn for an empty map", () => {
    expect(paintOverlay(fakeContext() as never, target(), [])).toEqual({
      drawn: 0,
      missing: 0,
      maxIndex: -1,
    });
  });

  it("counts the tiles it drew", () => {
    const report = paintOverlay(fakeContext() as never, target(), [
      { row: 0, col: 0, tile: 1, flags: noFlags, ch: "a" },
      { row: 0, col: 1, tile: 2, flags: noFlags, ch: "b" },
    ]);
    expect(report).toMatchObject({ drawn: 2, missing: 0 });
  });

  it("counts indices the sheet does not have", () => {
    // The signal that the sheet does not match the server's NetHack version.
    const report = paintOverlay(fakeContext() as never, target(), [
      { row: 0, col: 0, tile: 1, flags: noFlags, ch: "a" },
      { row: 0, col: 1, tile: 1272, flags: noFlags, ch: " " },
      { row: 0, col: 2, tile: 1292, flags: noFlags, ch: " " },
    ]);
    expect(report).toMatchObject({ drawn: 1, missing: 2 });
  });

  it("reports the highest index the server asked for", () => {
    // 5.0's "dark part of a room" is 1292; against a 1082-tile 3.6.7 sheet
    // that number is the evidence the wrong sheet is loaded.
    const report = paintOverlay(fakeContext() as never, target(), [
      { row: 0, col: 0, tile: 93, flags: noFlags, ch: "@" },
      { row: 0, col: 1, tile: 1292, flags: noFlags, ch: " " },
    ]);
    expect(report.maxIndex).toBe(1292);
  });

  it("does not count a tile that was skipped for being off-canvas", () => {
    const report = paintOverlay(fakeContext() as never, target({ widthPx: 40, heightPx: 40 }), [
      { row: 50, col: 50, tile: 1, flags: noFlags, ch: "a" },
    ]);
    expect(report.drawn).toBe(0);
  });
});
