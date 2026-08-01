import { describe, expect, it, vi } from "vitest";
import { StreamPlayer, type TerminalPort } from "./streamPlayer";
import { TileGrid } from "./tileGrid";
import type { GlyphFlags, StreamItem } from "./protocol";

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

/**
 * A terminal that behaves like xterm.js: writes are parsed asynchronously and
 * their callbacks fire in order. It tracks a cursor that advances one column
 * per printable byte, so tests can check *when* the cursor is read.
 */
function fakeTerminal() {
  const queue: { data: Uint8Array; callback?: () => void }[] = [];
  const written: number[] = [];
  let row = 0;
  let col = 0;

  const port: TerminalPort = {
    write(data, callback) {
      queue.push({ data, callback });
    },
    cursor: () => ({ row, col }),
  };

  return {
    port,
    /** Parses everything queued, running callbacks in order. */
    drain() {
      while (queue.length > 0) {
        const { data, callback } = queue.shift()!;
        for (const byte of data) {
          written.push(byte);
          if (byte === 0x0a) {
            row++;
            col = 0;
          } else {
            col++;
          }
        }
        callback?.();
      }
    },
    text: () => String.fromCharCode(...written),
    setCursor(r: number, c: number) {
      row = r;
      col = c;
    },
  };
}

const text = (s: string): StreamItem => ({ type: "text", bytes: s });
const glyph = (tile: number, flags = noFlags): StreamItem => ({
  type: "event",
  event: { kind: "glyphStart", tile, flags, rawFlags: 0 },
});
const glyphEnd: StreamItem = { type: "event", event: { kind: "glyphEnd" } };
const frameSync: StreamItem = { type: "event", event: { kind: "frameSync" } };

describe("StreamPlayer", () => {
  it("writes plain text through to the terminal unchanged", () => {
    const term = fakeTerminal();
    const player = new StreamPlayer(term.port, new TileGrid(), () => {});

    player.feed([text("hello")]);
    term.drain();

    expect(term.text()).toBe("hello");
  });

  it("preserves high bytes that are not valid UTF-8", () => {
    const term = fakeTerminal();
    const player = new StreamPlayer(term.port, new TileGrid(), () => {});

    // DECgraphics line drawing.
    player.feed([text("Ä³")]);
    term.drain();

    expect(term.text()).toBe("Ä³");
  });

  it("places a tile at the cursor position after the preceding text", () => {
    const term = fakeTerminal();
    const grid = new TileGrid();
    const player = new StreamPlayer(term.port, grid, () => {});

    // Five characters, then a glyph: the glyph belongs in column 5.
    player.feed([text("abcde"), glyph(344), text("d"), glyphEnd]);
    term.drain();
    grid.resolve(() => "d");

    expect(grid.get(0, 5)?.tile).toBe(344);
  });

  it("does not read the cursor until the terminal has caught up", () => {
    const term = fakeTerminal();
    const grid = new TileGrid();
    const player = new StreamPlayer(term.port, grid, () => {});
    const cursorSpy = vi.spyOn(term.port, "cursor");

    player.feed([text("abcde"), glyph(1)]);
    // Nothing has been parsed yet, so nothing may have been sampled.
    expect(cursorSpy).not.toHaveBeenCalled();

    term.drain();
    expect(cursorSpy).toHaveBeenCalledTimes(1);
  });

  it("places consecutive glyphs in consecutive cells", () => {
    const term = fakeTerminal();
    const grid = new TileGrid();
    const player = new StreamPlayer(term.port, grid, () => {});

    player.feed([
      glyph(10),
      text("-"),
      glyphEnd,
      glyph(11),
      text("|"),
      glyphEnd,
      glyph(12),
      text("."),
      glyphEnd,
    ]);
    term.drain();
    grid.resolve(() => "x");

    expect(grid.get(0, 0)?.tile).toBe(10);
    expect(grid.get(0, 1)?.tile).toBe(11);
    expect(grid.get(0, 2)?.tile).toBe(12);
  });

  it("carries the decoded flags onto the placed tile", () => {
    const term = fakeTerminal();
    const grid = new TileGrid();
    const player = new StreamPlayer(term.port, grid, () => {});
    const pet = { ...noFlags, pet: true };

    player.feed([glyph(7, pet), text("d"), glyphEnd]);
    term.drain();
    grid.resolve(() => "d");

    expect(grid.get(0, 0)?.flags).toEqual(pet);
  });

  it("tracks the cursor across a newline", () => {
    const term = fakeTerminal();
    const grid = new TileGrid();
    const player = new StreamPlayer(term.port, grid, () => {});

    player.feed([text("ab\nxy"), glyph(5), text("@"), glyphEnd]);
    term.drain();
    grid.resolve(() => "@");

    expect(grid.get(1, 2)?.tile).toBe(5);
  });

  it("signals a frame only once the terminal has processed the frame", () => {
    const term = fakeTerminal();
    const onFrame = vi.fn();
    const player = new StreamPlayer(term.port, new TileGrid(), onFrame);

    player.feed([text("map"), frameSync]);
    expect(onFrame).not.toHaveBeenCalled();

    term.drain();
    expect(onFrame).toHaveBeenCalledTimes(1);
  });

  it("signals each frame in a multi-frame batch", () => {
    const term = fakeTerminal();
    const onFrame = vi.fn();
    const player = new StreamPlayer(term.port, new TileGrid(), onFrame);

    player.feed([text("a"), frameSync, text("b"), frameSync]);
    term.drain();

    expect(onFrame).toHaveBeenCalledTimes(2);
  });

  it("keeps text and tile placement ordered across separate feeds", () => {
    const term = fakeTerminal();
    const grid = new TileGrid();
    const player = new StreamPlayer(term.port, grid, () => {});

    player.feed([text("abc")]);
    player.feed([glyph(42), text("@"), glyphEnd]);
    term.drain();
    grid.resolve(() => "@");

    expect(term.text()).toBe("abc@");
    expect(grid.get(0, 3)?.tile).toBe(42);
  });

  it("ignores events that need no terminal action", () => {
    const term = fakeTerminal();
    const player = new StreamPlayer(term.port, new TileGrid(), () => {});

    player.feed([
      { type: "event", event: { kind: "selectWindow", winid: 3 } },
      { type: "event", event: { kind: "sound", id: 7 } },
      glyphEnd,
      text("ok"),
    ]);
    term.drain();

    expect(term.text()).toBe("ok");
  });

  it("writes nothing at all for an empty batch", () => {
    const term = fakeTerminal();
    const writeSpy = vi.spyOn(term.port, "write");
    const player = new StreamPlayer(term.port, new TileGrid(), () => {});

    player.feed([]);

    expect(writeSpy).not.toHaveBeenCalled();
  });

  it("still signals a frame when the frame carried no text", () => {
    const term = fakeTerminal();
    const onFrame = vi.fn();
    const player = new StreamPlayer(term.port, new TileGrid(), onFrame);

    player.feed([frameSync]);
    term.drain();

    expect(onFrame).toHaveBeenCalledTimes(1);
  });
});
