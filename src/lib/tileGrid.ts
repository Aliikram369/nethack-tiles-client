/**
 * Tracks which terminal cells currently show a tile.
 *
 * The tricky part of a tile overlay is knowing when a tile becomes *stale*.
 * NetHack redraws the map incrementally, and message lines, menus and full
 * screen clears all paint over map cells without telling us to remove
 * anything.
 *
 * Two independent signals retire a tile, because neither is sufficient alone:
 *
 * - {@link damage}, the primary one. The caller knows which cells each write
 *   landed on, and a cell written by anything other than a map glyph is no
 *   longer showing that glyph. This is the only thing that catches an
 *   overwrite with the *same* character -- an unlit map cell and the gap
 *   between two words of a menu are both a space.
 * - the recorded character, as a backstop. A tile also goes if its cell no
 *   longer holds the character NetHack drew there, which covers anything that
 *   moves content around behind our back, such as a scroll or a resize.
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
  // Values carry their own coordinates: pruning walks every tile on every
  // batch of server output, and parsing them back out of the key each time
  // showed up as pure allocation.
  private tiles = new Map<string, TileEntry<F>>();
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
   * commits them.
   *
   * Call once the terminal has processed the character that follows each
   * start-glyph code. NetHack writes exactly one character between
   * `AVTC_GLYPH_START` and `AVTC_GLYPH_END` (`tty_print_glyph` in
   * `win/tty/wintty.c`), so by the next glyph the character is on screen.
   * Reading it any later risks recording whatever was drawn *over* the cell
   * instead, which would anchor the tile to the very thing that should have
   * retired it.
   */
  commit(readCell: CellReader): void {
    for (const { row, col, tile, flags } of this.pending) {
      const ch = readCell(row, col);
      if (ch === null) {
        // The terminal shrank out from under us.
        this.tiles.delete(key(row, col));
        continue;
      }
      this.tiles.set(key(row, col), { row, col, tile, flags, ch });
    }
    this.pending.length = 0;
  }

  /**
   * Retires the tile at a cell that something has written to.
   *
   * Also discards any glyph still waiting to be anchored there. A glyph is
   * placed as soon as its start code arrives but anchored a little later, and
   * without this a write that lands in between would be undone when the
   * anchor finally resolved -- putting the tile back on top of whatever
   * covered it.
   */
  damage(row: number, col: number): void {
    this.tiles.delete(key(row, col));
    this.pending = this.pending.filter((p) => p.row !== row || p.col !== col);
  }

  /** Drops every tile whose cell no longer holds the character it recorded. */
  prune(readCell: CellReader): void {
    for (const [k, placed] of this.tiles) {
      if (readCell(placed.row, placed.col) !== placed.ch) {
        this.tiles.delete(k);
      }
    }
  }

  /** {@link commit} followed by {@link prune}; the end-of-batch settle. */
  resolve(readCell: CellReader): void {
    this.commit(readCell);
    this.prune(readCell);
  }

  /** Every tile currently on screen. */
  entries(): TileEntry<F>[] {
    return [...this.tiles.values()];
  }

  get(row: number, col: number): TileEntry<F> | undefined {
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
