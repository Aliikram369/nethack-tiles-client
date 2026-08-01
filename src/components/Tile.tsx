import { tileRect, type TilesetPayload } from "../lib/protocol";

/**
 * A single tile from the loaded sheet, used as interface ornament.
 *
 * The chrome is drawn from the same art the game is drawn with -- the door you
 * walk through to enter a dungeon is the door on the connect button -- so the
 * app looks like the thing it is for rather than like a settings dialog.
 */
export function Tile({
  tileset,
  index,
  size = 16,
  title,
}: {
  tileset: TilesetPayload | null;
  index: number;
  size?: number;
  title?: string;
}) {
  const rect = tileset ? tileRect(tileset.manifest, index) : null;
  if (!tileset || !rect) {
    return <span className="tile tile--absent" style={{ width: size, height: size }} />;
  }

  const zoom = size / rect.width;
  return (
    <span
      className="tile"
      role={title ? "img" : "presentation"}
      aria-label={title}
      title={title}
      style={{
        width: size,
        height: size,
        backgroundImage: `url(${tileset.dataUrl})`,
        backgroundPosition: `-${rect.x * zoom}px -${rect.y * zoom}px`,
        backgroundSize: `${tileset.manifest.columns * rect.width * zoom}px auto`,
      }}
    />
  );
}

/** Tile indices used by the interface (NetHack 3.6.7 ordering). */
export const TILES = {
  valkyrie: 348,
  wizard: 349,
  archeologist: 335,
  samurai: 346,
  littleDog: 16,
  kitten: 34,
  openDoor: 863,
  staircaseDown: 874,
  staircaseUp: 873,
  fountain: 881,
  corridor: 871,
  verticalWall: 851,
  horizontalWall: 852,
  gridBug: 117,
} as const;
