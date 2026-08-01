/**
 * Paints the tile overlay onto a canvas laid over the terminal.
 *
 * Kept free of DOM lookups and xterm internals so it can be unit tested: the
 * caller measures the terminal and hands over the geometry.
 */

import { tileRect, type GlyphFlags, type TilesetManifest } from "./protocol";

/** Everything the painter needs to know about where to draw. */
export interface OverlayTarget {
  /** The loaded tile sheet. */
  sheet: CanvasImageSource;
  manifest: TilesetManifest;
  /** Size of one terminal cell, in CSS pixels. */
  cellWidth: number;
  cellHeight: number;
  /** Size of the overlay canvas, in CSS pixels. */
  widthPx: number;
  heightPx: number;
}

export interface OverlayTile {
  row: number;
  col: number;
  tile: number;
  flags: GlyphFlags;
  ch: string;
}

/** What a paint actually managed to draw, so the UI can spot a bad sheet. */
export interface PaintReport {
  drawn: number;
  /** Tiles whose index is not in the sheet -- the version-mismatch signal. */
  missing: number;
  /** Highest index the server asked for, or -1 if none. */
  maxIndex: number;
}

export function paintOverlay(
  ctx: CanvasRenderingContext2D,
  target: OverlayTarget,
  tiles: readonly OverlayTile[],
): PaintReport {
  const report: PaintReport = { drawn: 0, missing: 0, maxIndex: -1 };
  const { manifest, cellWidth, cellHeight, widthPx, heightPx } = target;

  ctx.clearRect(0, 0, widthPx, heightPx);
  // NetHack tiles are pixel art; smoothing turns them to mush when the cell
  // size does not divide evenly by the tile size.
  ctx.imageSmoothingEnabled = false;

  for (const tile of tiles) {
    report.maxIndex = Math.max(report.maxIndex, tile.tile);
    const x = tile.col * cellWidth;
    const y = tile.row * cellHeight;
    if (x >= widthPx || y >= heightPx || x + cellWidth <= 0 || y + cellHeight <= 0) {
      continue;
    }

    const src = tileRect(manifest, tile.tile);
    if (!src) {
      report.missing++;
      drawMissingTile(ctx, x, y, cellWidth, cellHeight);
      continue;
    }
    report.drawn++;

    ctx.drawImage(
      target.sheet,
      src.x,
      src.y,
      src.width,
      src.height,
      x,
      y,
      cellWidth,
      cellHeight,
    );

    if (tile.flags.detected || tile.flags.invisible) {
      ctx.fillStyle = tile.flags.invisible
        ? "rgba(255, 255, 255, 0.25)"
        : "rgba(120, 180, 255, 0.30)";
      ctx.fillRect(x, y, cellWidth, cellHeight);
    }

    if (tile.flags.pet) {
      drawPetHeart(ctx, x, y, cellWidth, cellHeight);
    }
  }

  return report;
}

/**
 * A tile index the sheet does not cover means the tileset does not match the
 * server's NetHack version. Draw something obvious rather than a hole.
 */
function drawMissingTile(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
): void {
  ctx.fillStyle = "#3a1020";
  ctx.fillRect(x, y, w, h);
  ctx.strokeStyle = "#ff5577";
  ctx.strokeRect(x + 0.5, y + 0.5, w - 1, h - 1);
  ctx.fillStyle = "#ff99aa";
  ctx.font = `${Math.floor(h * 0.7)}px monospace`;
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.fillText("?", x + w / 2, y + h / 2);
}

/** The classic tty pet marker, drawn in the corner of the tile. */
function drawPetHeart(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
): void {
  const size = Math.max(3, Math.min(w, h) * 0.35);
  const cx = x + size * 0.7;
  const cy = y + h - size * 0.7;

  ctx.fillStyle = "#ff3355";
  ctx.beginPath();
  ctx.moveTo(cx, cy + size * 0.35);
  ctx.lineTo(cx - size * 0.5, cy - size * 0.1);
  ctx.lineTo(cx - size * 0.25, cy - size * 0.45);
  ctx.lineTo(cx, cy - size * 0.15);
  ctx.lineTo(cx + size * 0.25, cy - size * 0.45);
  ctx.lineTo(cx + size * 0.5, cy - size * 0.1);
  ctx.closePath();
  ctx.fill();
}
