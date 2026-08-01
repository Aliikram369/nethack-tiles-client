import { describe, expect, it } from "vitest";
import { TileGrid, type CellReader } from "./tileGrid";

// The grid is generic in its flag type; these tests use plain numbers to keep
// the fixtures readable.
const grid_ = () => new TileGrid<number>();

/** A fake terminal buffer: rows of strings, read by (row, col). */
function screen(rows: string[]): CellReader {
  return (row, col) => {
    const line = rows[row];
    if (line === undefined || col < 0 || col >= line.length) return null;
    return line[col];
  };
}

describe("TileGrid", () => {
  it("starts empty", () => {
    expect(grid_().size).toBe(0);
    expect(grid_().entries()).toEqual([]);
  });

  it("does not expose a tile until the frame is resolved", () => {
    const grid = grid_();
    grid.place(2, 3, 344, 0);
    expect(grid.size).toBe(0);
  });

  it("records the character the terminal drew for the glyph", () => {
    const grid = grid_();
    grid.place(2, 3, 344, 16);
    grid.resolve(screen(["", "", "   d"]));

    expect(grid.get(2, 3)).toEqual({ row: 2, col: 3, tile: 344, flags: 16, ch: "d" });
  });

  it("drops a tile once its cell holds a different character", () => {
    const grid = grid_();
    grid.place(2, 3, 344, 0);
    grid.resolve(screen(["", "", "   d"]));
    expect(grid.size).toBe(1);

    // A message line or menu paints over the map.
    grid.resolve(screen(["", "", "   X"]));
    expect(grid.size).toBe(0);
  });

  it("keeps a tile whose cell is unchanged", () => {
    const grid = grid_();
    grid.place(2, 3, 344, 0);
    grid.resolve(screen(["", "", "   d"]));
    grid.resolve(screen(["", "", "   d"]));

    expect(grid.get(2, 3)?.tile).toBe(344);
  });

  it("drops every tile when the screen is cleared", () => {
    const grid = grid_();
    grid.place(1, 0, 10, 0);
    grid.place(1, 1, 11, 0);
    grid.resolve(screen(["", "@#"]));
    expect(grid.size).toBe(2);

    grid.resolve(screen(["", "  "]));
    expect(grid.size).toBe(0);
  });

  it("replaces the tile at a cell that gets a new glyph", () => {
    const grid = grid_();
    grid.place(1, 1, 10, 0);
    grid.resolve(screen(["", " @"]));

    grid.place(1, 1, 99, 4);
    grid.resolve(screen(["", " d"]));

    expect(grid.get(1, 1)).toEqual({ row: 1, col: 1, tile: 99, flags: 4, ch: "d" });
    expect(grid.size).toBe(1);
  });

  it("drops a glyph placed outside the terminal bounds", () => {
    const grid = grid_();
    grid.place(99, 99, 5, 0);
    grid.resolve(screen(["short"]));

    expect(grid.size).toBe(0);
  });

  it("drops tiles that fall outside the terminal after a resize", () => {
    const grid = grid_();
    grid.place(3, 10, 7, 0);
    grid.resolve(screen(["", "", "", "          #"]));
    expect(grid.size).toBe(1);

    // The terminal got narrower; that cell no longer exists.
    grid.resolve(screen(["", "", "", "###"]));
    expect(grid.size).toBe(0);
  });

  it("reports every placed tile with its coordinates", () => {
    const grid = grid_();
    grid.place(1, 0, 10, 0);
    grid.place(2, 4, 11, 8);
    grid.resolve(screen(["", "#", "    d"]));

    expect(grid.entries().sort((a, b) => a.row - b.row)).toEqual([
      { row: 1, col: 0, tile: 10, flags: 0, ch: "#" },
      { row: 2, col: 4, tile: 11, flags: 8, ch: "d" },
    ]);
  });

  it("forgets everything on clear", () => {
    const grid = grid_();
    grid.place(1, 1, 10, 0);
    grid.resolve(screen(["", " @"]));
    grid.clear();

    expect(grid.size).toBe(0);
    // A pending glyph is discarded too, not resurrected by the next resolve.
    grid.place(1, 1, 10, 0);
    grid.clear();
    grid.resolve(screen(["", " @"]));
    expect(grid.size).toBe(0);
  });

  it("drops the tile in a cell that something else wrote to", () => {
    const grid = grid_();
    grid.place(1, 1, 10, 0);
    grid.resolve(screen(["", " @"]));

    grid.damage(1, 1);

    expect(grid.size).toBe(0);
  });

  it("leaves neighbouring tiles alone when one cell is written to", () => {
    const grid = grid_();
    grid.place(1, 0, 10, 0);
    grid.place(1, 1, 11, 0);
    grid.resolve(screen(["", "@#"]));

    grid.damage(1, 1);

    expect(grid.size).toBe(1);
    expect(grid.get(1, 0)?.tile).toBe(10);
  });

  it("drops a tile overwritten by the same character it already held", () => {
    // The reason damage exists. An unlit map cell is drawn as a space, and so
    // is the gap between two words of a menu drawn on top of it, so comparing
    // characters can never tell the two apart -- the tile would survive and be
    // painted over the menu.
    const grid = grid_();
    grid.place(1, 1, 2360, 0);
    grid.resolve(screen(["", "  "]));
    expect(grid.size).toBe(1);

    grid.damage(1, 1);
    grid.resolve(screen(["", "  "]));

    expect(grid.size).toBe(0);
  });

  it("drops a glyph that is written over before it is anchored", () => {
    const grid = grid_();
    grid.place(1, 1, 10, 0);
    grid.damage(1, 1);
    grid.resolve(screen(["", " X"]));

    expect(grid.size).toBe(0);
  });

  it("ignores damage to a cell that never had a tile", () => {
    const grid = grid_();
    expect(() => grid.damage(4, 4)).not.toThrow();
    expect(grid.size).toBe(0);
  });

  it("survives a blank cell, which the terminal reports as a space", () => {
    const grid = grid_();
    grid.place(1, 1, 2360, 0);
    grid.resolve(screen(["", "  "]));

    expect(grid.get(1, 1)).toEqual({ row: 1, col: 1, tile: 2360, flags: 0, ch: " " });
    grid.resolve(screen(["", "  "]));
    expect(grid.size).toBe(1);
  });
});
