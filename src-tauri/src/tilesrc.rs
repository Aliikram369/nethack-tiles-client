//! Parser for NetHack's textual tile sources (`win/share/*.txt`).
//!
//! Those files are the canonical definition of the vanilla ("Amiga") 16x16
//! tileset. Each begins with a colour map and then a run of tile blocks:
//!
//! ```text
//! . = (71, 108, 108)
//! A = (0, 0, 0)
//! # tile 0 (giant ant)
//! {
//!   ................
//!   .......JAJKKA...
//!   ...
//! }
//! ```
//!
//! Tile indices are positional: concatenating `monsters.txt`, `objects.txt`
//! and `other.txt` in that order reproduces the ordering the server compiles
//! into `glyph2tile`, which is what `AVTC_GLYPH_START` indexes.

use std::collections::HashMap;

/// One parsed tile: RGBA pixels in row-major order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileImage {
    /// The descriptive name from the `# tile N (name)` header.
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// `width * height` RGBA pixels.
    pub pixels: Vec<[u8; 4]>,
}

impl TileImage {
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        self.pixels[(y * self.width + x) as usize]
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TileSourceError {
    #[error("line {line}: malformed colour definition {text:?}")]
    BadColor { line: usize, text: String },
    #[error("line {line}: character {ch:?} is not in the colour map")]
    UnknownColor { line: usize, ch: char },
    #[error("line {line}: tile {name:?} has a row of {got} pixels, expected {expected}")]
    RaggedRow {
        line: usize,
        name: String,
        got: usize,
        expected: usize,
    },
    #[error("line {line}: tile {name:?} has {got} rows, expected {expected}")]
    WrongHeight {
        line: usize,
        name: String,
        got: usize,
        expected: usize,
    },
    #[error("line {line}: unexpected {text:?} outside a tile block")]
    Unexpected { line: usize, text: String },
    #[error("tile {name:?} is never closed")]
    Unterminated { name: String },
    #[error("no tiles found")]
    Empty,
}

/// Parses one tile source file.
///
/// Tile dimensions are taken from the first block; every later block must
/// match, which catches a truncated or mis-edited source file.
pub fn parse_tile_file(text: &str) -> Result<Vec<TileImage>, TileSourceError> {
    let mut colors: HashMap<char, [u8; 4]> = HashMap::new();
    let mut tiles: Vec<TileImage> = Vec::new();

    // Set by the first complete tile; every later tile must agree.
    let mut dimensions: Option<(u32, u32)> = None;

    // The tile currently being read, if we are inside a `{ ... }` block.
    let mut open: Option<(String, Vec<Vec<[u8; 4]>>)> = None;
    let mut pending_name: Option<String> = None;

    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let trimmed = raw.trim();

        if let Some((name, rows)) = open.as_mut() {
            if trimmed == "}" {
                let (name, rows) = open.take().expect("just matched");
                let height = rows.len();
                let width = rows.first().map(|r| r.len()).unwrap_or(0);
                match dimensions {
                    None => dimensions = Some((width as u32, height as u32)),
                    Some((w, h)) => {
                        if height != h as usize {
                            return Err(TileSourceError::WrongHeight {
                                line,
                                name,
                                got: height,
                                expected: h as usize,
                            });
                        }
                        if width != w as usize {
                            return Err(TileSourceError::RaggedRow {
                                line,
                                name,
                                got: width,
                                expected: w as usize,
                            });
                        }
                    }
                }
                tiles.push(TileImage {
                    name,
                    width: width as u32,
                    height: height as u32,
                    pixels: rows.into_iter().flatten().collect(),
                });
                continue;
            }

            if trimmed.is_empty() {
                continue;
            }

            let mut row = Vec::with_capacity(trimmed.len());
            for ch in trimmed.chars() {
                let color = colors
                    .get(&ch)
                    .copied()
                    .ok_or(TileSourceError::UnknownColor { line, ch })?;
                row.push(color);
            }
            // Compare against the first row of this tile, then against the
            // established tile width once one exists.
            let expected = rows
                .first()
                .map(|r| r.len())
                .or_else(|| dimensions.map(|(w, _)| w as usize));
            if let Some(expected) = expected {
                if row.len() != expected {
                    return Err(TileSourceError::RaggedRow {
                        line,
                        name: name.clone(),
                        got: row.len(),
                        expected,
                    });
                }
            }
            rows.push(row);
            continue;
        }

        if trimmed.is_empty() {
            continue;
        }

        if trimmed == "{" {
            let name = pending_name.take().unwrap_or_default();
            open = Some((name, Vec::new()));
            continue;
        }

        if let Some(name) = tile_header_name(trimmed) {
            pending_name = Some(name);
            continue;
        }

        if trimmed.starts_with('#') {
            continue; // an ordinary comment
        }

        if let Some((ch, color)) = color_definition(trimmed) {
            colors.insert(ch, color);
            continue;
        }

        return Err(TileSourceError::Unexpected {
            line,
            text: trimmed.to_string(),
        });
    }

    if let Some((name, _)) = open {
        return Err(TileSourceError::Unterminated { name });
    }
    if tiles.is_empty() {
        return Err(TileSourceError::Empty);
    }
    Ok(tiles)
}

/// Lays tiles out row-major into a single sheet image.
///
/// The layout must match [`crate::tileset::Tileset::tile_rect`]: tile `n` sits
/// at column `n % columns`, row `n / columns`. Cells past the last tile are
/// left fully transparent.
pub fn compose_sheet(
    tiles: &[TileImage],
    columns: u32,
) -> Result<image::RgbaImage, TileSourceError> {
    let first = tiles.first().ok_or(TileSourceError::Empty)?;
    if columns == 0 {
        return Err(TileSourceError::Empty);
    }
    let (tw, th) = (first.width, first.height);

    for tile in tiles {
        if tile.width != tw {
            return Err(TileSourceError::RaggedRow {
                line: 0,
                name: tile.name.clone(),
                got: tile.width as usize,
                expected: tw as usize,
            });
        }
        if tile.height != th {
            return Err(TileSourceError::WrongHeight {
                line: 0,
                name: tile.name.clone(),
                got: tile.height as usize,
                expected: th as usize,
            });
        }
    }

    let rows = (tiles.len() as u32).div_ceil(columns);
    let mut sheet = image::RgbaImage::new(columns * tw, rows * th);
    for (index, tile) in tiles.iter().enumerate() {
        let ox = (index as u32 % columns) * tw;
        let oy = (index as u32 / columns) * th;
        for y in 0..th {
            for x in 0..tw {
                sheet.put_pixel(ox + x, oy + y, image::Rgba(tile.pixel(x, y)));
            }
        }
    }
    Ok(sheet)
}

/// Parses `# tile 12 (giant ant)` into its name.
fn tile_header_name(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix('#')?.trim_start();
    let rest = rest.strip_prefix("tile")?.trim_start();
    let open = rest.find('(')?;
    let close = rest.rfind(')')?;
    if close <= open {
        return None;
    }
    Some(rest[open + 1..close].to_string())
}

/// Parses `A = (0, 182, 255)` into its character and colour.
fn color_definition(line: &str) -> Option<(char, [u8; 4])> {
    let (lhs, rhs) = line.split_once('=')?;
    let mut chars = lhs.trim().chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let rhs = rhs.trim().strip_prefix('(')?.strip_suffix(')')?;
    let parts: Vec<_> = rhs.split(',').map(|p| p.trim().parse::<u8>()).collect();
    if parts.len() != 3 {
        return None;
    }
    let mut rgb = [0u8; 3];
    for (slot, part) in rgb.iter_mut().zip(parts) {
        *slot = part.ok()?;
    }
    Some((ch, [rgb[0], rgb[1], rgb[2], 255]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TWO_TILES: &str = "\
. = (71, 108, 108)
A = (0, 0, 0)
B = (255, 0, 0)
# tile 0 (giant ant)
{
  ....
  .AA.
  .BB.
  ....
}
# tile 1 (killer bee)
{
  BBBB
  BAAB
  BAAB
  BBBB
}
";

    #[test]
    fn parses_a_colour_definition() {
        assert_eq!(color_definition("A = (0, 182, 255)"), Some(('A', [0, 182, 255, 255])));
        assert_eq!(color_definition("U = (205,205,205)"), Some(('U', [205, 205, 205, 255])));
        assert_eq!(color_definition(". = (71, 108, 108)"), Some(('.', [71, 108, 108, 255])));
        assert_eq!(color_definition("not a colour"), None);
        assert_eq!(color_definition("AB = (1, 2, 3)"), None);
        assert_eq!(color_definition("A = (1, 2)"), None);
    }

    #[test]
    fn parses_a_tile_header() {
        assert_eq!(tile_header_name("# tile 0 (giant ant)"), Some("giant ant".into()));
        assert_eq!(
            tile_header_name("# tile 314 (lit corridor)"),
            Some("lit corridor".into())
        );
        assert_eq!(tile_header_name("# not a tile"), None);
    }

    #[test]
    fn parses_every_tile_in_order() {
        let tiles = parse_tile_file(TWO_TILES).expect("parse");
        assert_eq!(tiles.len(), 2);
        assert_eq!(tiles[0].name, "giant ant");
        assert_eq!(tiles[1].name, "killer bee");
    }

    #[test]
    fn tile_dimensions_come_from_the_source() {
        let tiles = parse_tile_file(TWO_TILES).unwrap();
        assert_eq!((tiles[0].width, tiles[0].height), (4, 4));
        assert_eq!(tiles[0].pixels.len(), 16);
    }

    #[test]
    fn pixels_take_their_colour_from_the_colour_map() {
        let tiles = parse_tile_file(TWO_TILES).unwrap();
        let ant = &tiles[0];
        assert_eq!(ant.pixel(0, 0), [71, 108, 108, 255]); // '.'
        assert_eq!(ant.pixel(1, 1), [0, 0, 0, 255]); // 'A'
        assert_eq!(ant.pixel(1, 2), [255, 0, 0, 255]); // 'B'
    }

    #[test]
    fn pixels_are_stored_row_major() {
        let tiles = parse_tile_file(TWO_TILES).unwrap();
        let bee = &tiles[1];
        // Row 1 is B A A B, so (0,1) differs from (1,1).
        assert_eq!(bee.pixel(0, 1), [255, 0, 0, 255]);
        assert_eq!(bee.pixel(1, 1), [0, 0, 0, 255]);
    }

    #[test]
    fn an_unknown_colour_character_is_rejected() {
        let src = ". = (1, 2, 3)\n# tile 0 (x)\n{\n  .?\n  ..\n}\n";
        assert!(matches!(
            parse_tile_file(src),
            Err(TileSourceError::UnknownColor { ch: '?', .. })
        ));
    }

    #[test]
    fn a_ragged_row_is_rejected() {
        let src = ". = (1, 2, 3)\n# tile 0 (x)\n{\n  ....\n  ..\n  ....\n  ....\n}\n";
        assert!(
            matches!(parse_tile_file(src), Err(TileSourceError::RaggedRow { .. })),
            "a short row must not be silently padded"
        );
    }

    #[test]
    fn a_tile_of_the_wrong_height_is_rejected() {
        // Second tile has 3 rows where the first established 4.
        let src = format!("{TWO_TILES}# tile 2 (short)\n{{\n  ....\n  ....\n  ....\n}}\n");
        assert!(matches!(
            parse_tile_file(&src),
            Err(TileSourceError::WrongHeight { .. })
        ));
    }

    #[test]
    fn an_unterminated_tile_is_rejected() {
        let src = ". = (1, 2, 3)\n# tile 0 (x)\n{\n  ..\n";
        assert!(matches!(
            parse_tile_file(src),
            Err(TileSourceError::Unterminated { .. })
        ));
    }

    #[test]
    fn a_file_with_no_tiles_is_rejected() {
        assert_eq!(parse_tile_file(". = (1, 2, 3)\n"), Err(TileSourceError::Empty));
    }

    /// A solid tile of one colour, for layout assertions.
    fn solid(name: &str, color: [u8; 4]) -> TileImage {
        TileImage {
            name: name.into(),
            width: 4,
            height: 4,
            pixels: vec![color; 16],
        }
    }

    #[test]
    fn the_sheet_is_columns_wide_and_tall_enough_for_every_tile() {
        let tiles: Vec<_> = (0..5).map(|i| solid(&i.to_string(), [1, 2, 3, 255])).collect();
        let sheet = compose_sheet(&tiles, 3).expect("compose");
        // 5 tiles at 3 per row -> 2 rows.
        assert_eq!(sheet.dimensions(), (3 * 4, 2 * 4));
    }

    #[test]
    fn tiles_are_placed_where_tile_rect_says_they_are() {
        let red = [255, 0, 0, 255];
        let green = [0, 255, 0, 255];
        let blue = [0, 0, 255, 255];
        let sheet = compose_sheet(
            &[solid("r", red), solid("g", green), solid("b", blue)],
            2,
        )
        .unwrap();

        // index 0 -> (col 0, row 0); 1 -> (col 1, row 0); 2 -> (col 0, row 1).
        assert_eq!(sheet.get_pixel(1, 1).0, red);
        assert_eq!(sheet.get_pixel(4 + 1, 1).0, green);
        assert_eq!(sheet.get_pixel(1, 4 + 1).0, blue);
    }

    #[test]
    fn unused_cells_in_the_last_row_are_transparent() {
        let sheet = compose_sheet(&[solid("a", [255, 0, 0, 255])], 2).unwrap();
        assert_eq!(sheet.get_pixel(4 + 1, 1).0, [0, 0, 0, 0]);
    }

    #[test]
    fn composing_no_tiles_is_an_error() {
        assert_eq!(compose_sheet(&[], 4), Err(TileSourceError::Empty));
    }

    #[test]
    fn a_sheet_composed_from_a_parsed_file_round_trips_through_the_tileset_loader() {
        use crate::glyph::NetHackVersion;
        use crate::tileset::{Tileset, TilesetManifest};

        let tiles = parse_tile_file(TWO_TILES).unwrap();
        let columns = 2;
        let sheet = compose_sheet(&tiles, columns).unwrap();

        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(sheet)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();

        let manifest = TilesetManifest {
            id: "generated".into(),
            name: "Generated".into(),
            version: NetHackVersion::V36,
            tile_width: tiles[0].width,
            tile_height: tiles[0].height,
            columns,
            tile_count: tiles.len() as u32,
        };
        let loaded = Tileset::load(manifest, png.into_inner())
            .expect("a freshly composed sheet must satisfy the loader");
        assert!(loaded.tile_rect(1).is_some());
        assert_eq!(loaded.tile_rect(2), None);
    }

    #[test]
    fn blank_lines_and_comments_between_tiles_are_tolerated() {
        let src = "\
. = (1, 2, 3)

# a stray comment

# tile 0 (x)
{
  ..
  ..
}

";
        assert_eq!(parse_tile_file(src).unwrap().len(), 1);
    }
}
