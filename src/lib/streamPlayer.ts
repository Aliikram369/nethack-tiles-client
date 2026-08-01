/**
 * Replays the backend's ordered stream into the terminal, placing tiles at the
 * cells NetHack meant them for.
 *
 * The cursor is the crux. In `tty_print_glyph` NetHack moves the cursor with
 * `tty_curs` *before* emitting the start-glyph escape code, then writes the
 * character. So the target cell is wherever the cursor sits once every byte
 * preceding the escape code has been processed -- which means we must ask the
 * terminal, after it has caught up, rather than guess.
 *
 * xterm.js invokes `write`'s callback once *that* chunk has been parsed, and
 * callbacks fire in write order, so flushing the pending text with a callback
 * gives us exactly the right moment to read the cursor. This is why the
 * backend hands us an ordered stream instead of pre-computed coordinates: it
 * would otherwise have to reimplement a terminal emulator to know where the
 * cursor is.
 */

import { latin1ToBytes, type GlyphFlags, type StreamItem } from "./protocol";
import type { TileGrid } from "./tileGrid";

/** The slice of xterm.js this module depends on. */
export interface TerminalPort {
  write(data: Uint8Array, callback?: () => void): void;
  /** Cursor position in viewport coordinates. */
  cursor(): { row: number; col: number };
}

export class StreamPlayer {
  private pending: Uint8Array[] = [];

  constructor(
    private readonly term: TerminalPort,
    private readonly grid: TileGrid<GlyphFlags>,
    /** Called once the terminal has caught up with a frame boundary. */
    private readonly onFrame: () => void,
  ) {}

  feed(items: readonly StreamItem[]): void {
    for (const item of items) {
      if (item.type === "text") {
        this.pending.push(latin1ToBytes(item.bytes));
        continue;
      }

      const event = item.event;
      switch (event.kind) {
        case "glyphStart": {
          const { tile, flags } = event;
          this.flush(() => {
            const { row, col } = this.term.cursor();
            this.grid.place(row, col, tile, flags);
          });
          break;
        }
        case "frameSync":
          this.flush(() => this.onFrame());
          break;
        // glyphEnd, selectWindow and sound need no terminal action: a glyph is
        // only ever emitted for the map, so glyphStart alone identifies it.
        default:
          break;
      }
    }
    this.flush();
  }

  /**
   * Writes everything buffered so far, running `callback` once the terminal
   * has processed exactly that much of the stream.
   */
  private flush(callback?: () => void): void {
    const data = concat(this.pending);
    this.pending = [];
    if (data.length === 0 && !callback) return;
    this.term.write(data, callback);
  }
}

function concat(chunks: readonly Uint8Array[]): Uint8Array {
  if (chunks.length === 1) return chunks[0];
  let total = 0;
  for (const c of chunks) total += c.length;
  const out = new Uint8Array(total);
  let at = 0;
  for (const c of chunks) {
    out.set(c, at);
    at += c.length;
  }
  return out;
}
