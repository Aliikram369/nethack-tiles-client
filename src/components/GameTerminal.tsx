import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import "@xterm/xterm/css/xterm.css";

import { paintOverlay } from "../lib/overlay";
import type { Profile, TilesetPayload } from "../lib/protocol";
import { StreamPlayer } from "../lib/streamPlayer";
import { TileGrid } from "../lib/tileGrid";
import { onStream, sshResize, sshWrite } from "../lib/tauri";

/**
 * The NetHack colour scheme, kept close to a classic 16-colour terminal so
 * ASCII fallback looks right when a tile is missing.
 */
const THEME = {
  background: "#0d1416",
  foreground: "#cfc9b8",
  cursor: "#ffb642",
  black: "#0d1416",
  red: "#c33b3b",
  green: "#3f9e4d",
  yellow: "#c98a2b",
  blue: "#3b6fc3",
  magenta: "#a552b5",
  cyan: "#2f979b",
  white: "#b8b2a2",
  brightBlack: "#476c6c",
  brightRed: "#ff5555",
  brightGreen: "#5ddb6d",
  brightYellow: "#ffb642",
  brightBlue: "#6fa2ff",
  brightMagenta: "#d77bea",
  brightCyan: "#4fd6da",
  brightWhite: "#f2ece0",
};

interface Props {
  profile: Profile;
  tileset: TilesetPayload | null;
  /** Draw tiles over the map, or leave plain ASCII. */
  tilesEnabled: boolean;
}

export function GameTerminal({ profile, tileset, tilesEnabled }: Props) {
  const hostRef = useRef<HTMLDivElement>(null);
  // Kept in refs so the render loop reads current values without re-creating
  // the terminal, which would drop the session's scrollback.
  const tilesetRef = useRef(tileset);
  const enabledRef = useRef(tilesEnabled);
  const repaintRef = useRef<() => void>(() => {});

  tilesetRef.current = tileset;
  enabledRef.current = tilesEnabled;

  // Repaint when the tileset or the toggle changes, without a full rebuild.
  useEffect(() => {
    repaintRef.current();
  }, [tileset, tilesEnabled]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const term = new Terminal({
      fontFamily: profile.fontFamily,
      fontSize: Math.round(profile.fontSize * profile.scale),
      theme: THEME,
      cursorBlink: true,
      // NetHack repaints in place; scrollback only gets in the way and would
      // desynchronise the overlay's row coordinates.
      scrollback: 0,
      allowProposedApi: true,
    });

    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);

    try {
      term.loadAddon(new WebglAddon());
    } catch {
      // WebGL is unavailable in some webviews; the DOM renderer still works.
    }

    const canvas = document.createElement("canvas");
    canvas.className = "tile-overlay";
    const xtermEl = host.querySelector<HTMLElement>(".xterm");
    xtermEl?.appendChild(canvas);
    const ctx = canvas.getContext("2d");

    const grid = new TileGrid();

    /** Reads a cell from xterm's buffer; `null` means "outside the screen". */
    const readCell = (row: number, col: number): string | null => {
      if (row < 0 || row >= term.rows || col < 0 || col >= term.cols) return null;
      const buffer = term.buffer.active;
      const line = buffer.getLine(buffer.baseY + row);
      if (!line) return null;
      const chars = line.getCell(col)?.getChars() ?? "";
      // xterm reports an untouched cell as the empty string; NetHack writing a
      // space must compare equal to it.
      return chars === "" ? " " : chars;
    };

    const port = {
      write: (data: Uint8Array, callback?: () => void) => term.write(data, callback),
      cursor: () => ({
        row: term.buffer.active.cursorY,
        col: term.buffer.active.cursorX,
      }),
    };

    let sheet: HTMLImageElement | null = null;
    let sheetForUrl: string | null = null;
    let frameRequested = false;

    const paint = () => {
      frameRequested = false;
      const screenEl = host.querySelector<HTMLElement>(".xterm-screen");
      if (!ctx || !xtermEl || !screenEl) return;

      const outer = xtermEl.getBoundingClientRect();
      const screen = screenEl.getBoundingClientRect();
      const dpr = window.devicePixelRatio || 1;

      canvas.style.left = `${screen.left - outer.left}px`;
      canvas.style.top = `${screen.top - outer.top}px`;
      canvas.style.width = `${screen.width}px`;
      canvas.style.height = `${screen.height}px`;
      const pixelWidth = Math.max(1, Math.round(screen.width * dpr));
      const pixelHeight = Math.max(1, Math.round(screen.height * dpr));
      if (canvas.width !== pixelWidth || canvas.height !== pixelHeight) {
        canvas.width = pixelWidth;
        canvas.height = pixelHeight;
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      const active = tilesetRef.current;
      if (!enabledRef.current || !active) {
        ctx.clearRect(0, 0, screen.width, screen.height);
        return;
      }

      if (sheetForUrl !== active.dataUrl) {
        sheetForUrl = active.dataUrl;
        const image = new Image();
        image.onload = () => {
          sheet = image;
          requestPaint();
        };
        image.src = active.dataUrl;
        sheet = null;
      }
      if (!sheet) {
        ctx.clearRect(0, 0, screen.width, screen.height);
        return;
      }

      paintOverlay(
        ctx,
        {
          sheet,
          manifest: active.manifest,
          cellWidth: screen.width / term.cols,
          cellHeight: screen.height / term.rows,
          widthPx: screen.width,
          heightPx: screen.height,
        },
        grid.entries(),
      );
    };

    const requestPaint = () => {
      if (frameRequested) return;
      frameRequested = true;
      requestAnimationFrame(paint);
    };
    repaintRef.current = requestPaint;

    const player = new StreamPlayer(port, grid, () => {
      grid.resolve(readCell);
      requestPaint();
    });

    const unlistenStream = onStream((items) => player.feed(items));
    const dataSub = term.onData((data) => {
      void sshWrite(data);
    });

    const applyFit = () => {
      fit.fit();
      void sshResize(term.cols, term.rows);
      // The cell geometry moved, so every tile's pixel rect is now wrong.
      grid.resolve(readCell);
      requestPaint();
    };

    const observer = new ResizeObserver(() => applyFit());
    observer.observe(host);
    applyFit();
    term.focus();

    return () => {
      observer.disconnect();
      dataSub.dispose();
      void unlistenStream.then((un) => un());
      term.dispose();
      canvas.remove();
    };
    // Rebuilding on font changes is intentional: xterm needs to re-measure.
  }, [profile.fontFamily, profile.fontSize, profile.scale]);

  return <div className="terminal-host" ref={hostRef} />;
}
