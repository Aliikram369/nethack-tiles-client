/**
 * Tracks which terminal cells currently show a tile.
 *
 * The tricky part of a tile overlay is knowing when a tile becomes *stale*.
 * NetHack redraws the map incrementally, and message lines, menus and full
 * screen clears all paint over map cells without telling us to remove
 * anything.
 *
 * Rather than emulating the terminal to work that out, this grid uses the
 * terminal buffer itself as the source of truth: when a tile is placed we
 * record the character NetHack drew in that cell, and on every frame we drop
 * any tile whose cell no longer holds that character. A menu, a message or a
 * clear-screen changes the character, so the tile disappears on its own; a
 * redraw of the same glyph writes the same character, so the tile survives.
 */

import type { GlyphFlags } from "./protocol";

/** Reads the character currently in a cell, or `null` if out of range. */
export type CellReader = (row: number, col: number) => string | null;

/**
 * Generic in the flag type: the grid is a pure data structure and has no
 * business knowing how NetHack's `MG_*` bitmask is decoded.
 */
export interface PlacedTile<F> {
  tile: number;
  flags: F;
  /** The character NetHack drew here, used to detect overwrites. */
  ch: string;
}

export interface TileEntry<F> extends PlacedTile<F> {
  row: number;
  col: number;
}

const key = (row: number, col: number) => `${row},${col}`;

export class TileGrid<F = GlyphFlags> {
  private tiles = new Map<string, PlacedTile<F>>();
  /** Glyphs seen this frame whose character has not been read back yet. */
  private pending: { row: number; col: number; tile: number; flags: F }[] = [];

  /**
   * Records a glyph at a cell. The character is not known yet -- NetHack
   * writes it immediately after the start-glyph escape code -- so this is
   * held until {@link resolve}.
   */
  place(row: number, col: number, tile: number, flags: F): void {
    this.pending.push({ row, col, tile, flags });
  }

  /**
   * Reads back the characters for glyphs placed since the last call and
   * commits them, then drops every tile whose cell has since changed.
   *
   * Call once per frame, after the terminal has processed the frame's output.
   */
  resolve(readCell: CellReader): void {
    for (const { row, col, tile, flags } of this.pending) {
      const ch = readCell(row, col);
      if (ch === null) {
        // The terminal shrank out from under us.
        this.tiles.delete(key(row, col));
        continue;
      }
      this.tiles.set(key(row, col), { tile, flags, ch });
    }
    this.pending.length = 0;

    for (const [k, placed] of this.tiles) {
      const [row, col] = k.split(",").map(Number);
      if (readCell(row, col) !== placed.ch) {
        this.tiles.delete(k);
      }
    }
  }

  /** Every tile currently on screen. */
  entries(): TileEntry<F>[] {
    const out: TileEntry<F>[] = [];
    for (const [k, placed] of this.tiles) {
      const [row, col] = k.split(",").map(Number);
      out.push({ row, col, ...placed });
    }
    return out;
  }

  get(row: number, col: number): PlacedTile<F> | undefined {
    return this.tiles.get(key(row, col));
  }

  get size(): number {
    return this.tiles.size;
  }

  clear(): void {
    this.tiles.clear();
    this.pending.length = 0;
  }
}
