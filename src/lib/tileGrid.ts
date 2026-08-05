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
   * The last terrain seen in a cell, kept so a square that goes dark can carry
   * on showing what the player found there. See {@link commit}.
   */
  private terrain = new Map<string, TileEntry<F>>();
  /**
   * Lowest tile index ever drawn as a space, which is `S_stone` -- the first
   * entry of NetHack's terrain block, since the tile file runs monsters, then
   * objects, then terrain. So a lower index is a monster or an object and must
   * not be mistaken for terrain. Learned rather than hardcoded because the
   * numbering moves between NetHack versions.
   */
  private stoneTile: number | null = null;

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
      const k = key(row, col);
      const ch = readCell(row, col);
      if (ch === null) {
        // The terminal shrank out from under us.
        this.tiles.delete(k);
        this.terrain.delete(k);
        continue;
      }

      if (ch === " ") {
        // NetHack is showing the player nothing here. That is `S_stone`, which
        // means three things at once: the rock a corridor is cut through, a
        // square never seen, and -- the common one -- a floor square the hero
        // can no longer see. Its tile is an opaque rock texture, so painting it
        // turns every room already walked through into solid rock.
        //
        // Nothing in the glyph tells the three apart, but the cell's own past
        // does: if terrain was ever found here, this is that terrain gone dark.
        this.stoneTile = this.stoneTile === null ? tile : Math.min(this.stoneTile, tile);
        const remembered = this.terrain.get(k);
        if (remembered && remembered.tile >= this.stoneTile) {
          // The recorded character has to become the space the cell now holds,
          // or the backstop in `prune` would drop the tile on the next frame.
          this.tiles.set(k, { ...remembered, ch });
          continue;
        }
        // Nothing was ever here, so it really is rock.
        this.tiles.set(k, { row, col, tile, flags, ch });
        continue;
      }

      const entry = { row, col, tile, flags, ch };
      this.tiles.set(k, entry);
      // A monster standing on a floor square must not become the memory of it.
      // Before the first space arrives there is no boundary to test against, so
      // record it anyway; the check on the way out settles it.
      if (this.stoneTile === null || tile >= this.stoneTile) {
        this.terrain.set(k, entry);
      }
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

  /**
   * Retires the tile at a cell *and* forgets what terrain was found there.
   *
   * For a cell NetHack says nothing is known about. {@link damage} is the
   * gentler one: a menu drawn over the map hides terrain without unlearning it,
   * and the memory has to survive that.
   */
  forget(row: number, col: number): void {
    this.terrain.delete(key(row, col));
    this.damage(row, col);
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
    // The remembered terrain goes too: a clear is a new level as often as not,
    // and stale memory would show the old one's floor through the new one's
    // rock. `stoneTile` survives, being a property of the tile file.
    this.terrain.clear();
  }
}
