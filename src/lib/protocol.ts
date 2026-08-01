/**
 * Shapes emitted by the Rust backend on the `nh://stream` event.
 *
 * These mirror `src-tauri/src/demux.rs`. Text arrives latin-1 encoded because
 * the NetHack stream is not valid UTF-8 in general -- IBMgraphics and
 * DECgraphics line drawing use bytes above 0x7f -- so each char maps to
 * exactly one byte.
 */

export type TileEvent =
  | { kind: "glyphStart"; tile: number; flags: GlyphFlags; rawFlags: number }
  | { kind: "glyphEnd" }
  | { kind: "selectWindow"; winid: number | null }
  | { kind: "frameSync" }
  | { kind: "sound"; id: number | null };

export type StreamItem =
  | { type: "text"; bytes: string }
  | { type: "event"; event: TileEvent };

/** Decoded `MG_*` glyph flags, as sent by the backend. */
export interface GlyphFlags {
  hero: boolean;
  corpse: boolean;
  invisible: boolean;
  detected: boolean;
  pet: boolean;
  ridden: boolean;
  statue: boolean;
  objpile: boolean;
  bwLava: boolean;
  female: boolean;
}

export interface TilesetManifest {
  id: string;
  name: string;
  version: "v36" | "v50";
  tileWidth: number;
  tileHeight: number;
  columns: number;
  tileCount: number;
}

export interface TilesetPayload {
  manifest: TilesetManifest;
  dataUrl: string;
}

export interface Profile {
  id: string;
  name: string;
  host: string;
  port: number;
  sshUser: string;
  gameUser: string;
  version: "v36" | "v50";
  tilesetId: string;
  fontFamily: string;
  fontSize: number;
  scale: number;
  autoLogin: boolean;
}

export type Status =
  | { state: "connecting"; message: string }
  | { state: "connected"; message: string }
  | { state: "info"; message: string }
  | { state: "error"; message: string }
  | { state: "closed"; message: string | null };

/**
 * Turns the backend's latin-1 string back into the exact bytes NetHack sent.
 *
 * `String.prototype.charCodeAt` returns the code point, which for latin-1 is
 * the byte value, so this is lossless for anything the backend produced.
 */
export function latin1ToBytes(text: string): Uint8Array {
  const bytes = new Uint8Array(text.length);
  for (let i = 0; i < text.length; i++) {
    bytes[i] = text.charCodeAt(i) & 0xff;
  }
  return bytes;
}

/** Where a tile lives in the sheet, given the sheet's geometry. */
export function tileRect(
  manifest: TilesetManifest,
  index: number,
): { x: number; y: number; width: number; height: number } | null {
  if (!Number.isInteger(index) || index < 0 || index >= manifest.tileCount) {
    return null;
  }
  return {
    x: (index % manifest.columns) * manifest.tileWidth,
    y: Math.floor(index / manifest.columns) * manifest.tileHeight,
    width: manifest.tileWidth,
    height: manifest.tileHeight,
  };
}
