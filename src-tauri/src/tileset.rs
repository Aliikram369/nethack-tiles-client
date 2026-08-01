//! Tile sheet loading and index -> sub-rectangle mapping.
//!
//! A tileset is a PNG grid plus a manifest describing its geometry. The tile
//! index in `AVTC_GLYPH_START` is `glyph2tile[glyph]`, a direct index into
//! this grid in row-major order.
//!
//! The ordering is compiled into the server's binary and changes between
//! NetHack releases, so each tileset records the version it was built for and
//! the UI refuses to silently pair a 5.0 sheet with a 3.6 server.

use serde::{Deserialize, Serialize};

use crate::glyph::NetHackVersion;

/// Geometry and provenance of a tile sheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TilesetManifest {
    /// Stable identifier used in profiles, e.g. `vanilla-3.6.7-16`.
    pub id: String,
    /// Human-readable name for the picker.
    pub name: String,
    /// NetHack release this sheet's tile ordering was built from.
    pub version: NetHackVersion,
    pub tile_width: u32,
    pub tile_height: u32,
    /// Tiles per row in the sheet.
    pub columns: u32,
    /// Number of tiles that actually carry an image. The last row may be
    /// partially filled.
    pub tile_count: u32,
}

/// Where a tile lives inside the sheet, in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TileRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum TilesetError {
    #[error("tile sheet is not a readable PNG: {0}")]
    Decode(String),
    #[error("manifest has a zero tile_width, tile_height or columns")]
    DegenerateGeometry,
    #[error(
        "tile sheet is {actual_width}x{actual_height} but the manifest \
         describes {expected_width}x{expected_height} \
         ({columns} columns of {tile_width}x{tile_height}, {rows} rows)"
    )]
    DimensionMismatch {
        actual_width: u32,
        actual_height: u32,
        expected_width: u32,
        expected_height: u32,
        columns: u32,
        rows: u32,
        tile_width: u32,
        tile_height: u32,
    },
}

/// A loaded, validated tile sheet.
#[derive(Debug, Clone)]
pub struct Tileset {
    manifest: TilesetManifest,
    png: Vec<u8>,
    rows: u32,
}

impl Tileset {
    /// Validates `png` against `manifest` and returns the loaded sheet.
    pub fn load(manifest: TilesetManifest, png: Vec<u8>) -> Result<Self, TilesetError> {
        if manifest.tile_width == 0 || manifest.tile_height == 0 || manifest.columns == 0 {
            return Err(TilesetError::DegenerateGeometry);
        }

        let decoded = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .map_err(|e| TilesetError::Decode(e.to_string()))?;
        let (actual_width, actual_height) = (decoded.width(), decoded.height());

        let rows = rows_for(manifest.tile_count, manifest.columns);
        let expected_width = manifest.columns * manifest.tile_width;
        let expected_height = rows * manifest.tile_height;

        // The width must match exactly or every index past the first row is
        // wrong. Extra height is tolerated: some generators pad the sheet.
        if actual_width != expected_width || actual_height < expected_height {
            return Err(TilesetError::DimensionMismatch {
                actual_width,
                actual_height,
                expected_width,
                expected_height,
                columns: manifest.columns,
                rows,
                tile_width: manifest.tile_width,
                tile_height: manifest.tile_height,
            });
        }

        Ok(Tileset {
            manifest,
            png,
            rows,
        })
    }

    pub fn manifest(&self) -> &TilesetManifest {
        &self.manifest
    }

    /// Number of tile rows in the sheet.
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// The sub-rectangle for `index`, or `None` if the sheet has no such tile.
    ///
    /// A `None` here means the sheet does not match the server's NetHack
    /// version; callers draw a placeholder and warn the user.
    pub fn tile_rect(&self, index: u32) -> Option<TileRect> {
        if index >= self.manifest.tile_count {
            return None;
        }
        Some(TileRect {
            x: (index % self.manifest.columns) * self.manifest.tile_width,
            y: (index / self.manifest.columns) * self.manifest.tile_height,
            width: self.manifest.tile_width,
            height: self.manifest.tile_height,
        })
    }

    /// The sheet as a `data:` URL the webview can use as an image source.
    pub fn data_url(&self) -> String {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(&self.png);
        format!("data:image/png;base64,{b64}")
    }
}

/// Number of rows needed to hold `tile_count` tiles at `columns` per row.
fn rows_for(tile_count: u32, columns: u32) -> u32 {
    tile_count.div_ceil(columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a real PNG of the given size so tests exercise actual decoding.
    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(width, height, image::Rgba([1, 2, 3, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("encoding a PNG should succeed");
        out.into_inner()
    }

    /// 40 columns x 16px, 1057 tiles -> 27 rows (last row partly empty).
    fn manifest() -> TilesetManifest {
        TilesetManifest {
            id: "test-16".into(),
            name: "Test 16x16".into(),
            version: NetHackVersion::V36,
            tile_width: 16,
            tile_height: 16,
            columns: 40,
            tile_count: 1057,
        }
    }

    fn loaded() -> Tileset {
        let m = manifest();
        let rows = rows_for(m.tile_count, m.columns);
        let png = png_bytes(m.columns * m.tile_width, rows * m.tile_height);
        Tileset::load(m, png).expect("a matching sheet should load")
    }

    #[test]
    fn rows_are_rounded_up_for_a_partial_last_row() {
        assert_eq!(rows_for(1057, 40), 27);
        assert_eq!(rows_for(80, 40), 2);
        assert_eq!(rows_for(1, 40), 1);
        assert_eq!(rows_for(0, 40), 0);
    }

    #[test]
    fn first_tile_is_at_the_origin() {
        assert_eq!(
            loaded().tile_rect(0),
            Some(TileRect {
                x: 0,
                y: 0,
                width: 16,
                height: 16
            })
        );
    }

    #[test]
    fn tiles_advance_across_a_row() {
        assert_eq!(loaded().tile_rect(3).map(|r| (r.x, r.y)), Some((48, 0)));
    }

    #[test]
    fn tile_index_wraps_to_the_next_row_at_the_column_count() {
        let ts = loaded();
        assert_eq!(ts.tile_rect(40).map(|r| (r.x, r.y)), Some((0, 16)));
        assert_eq!(ts.tile_rect(41).map(|r| (r.x, r.y)), Some((16, 16)));
        assert_eq!(ts.tile_rect(85).map(|r| (r.x, r.y)), Some((5 * 16, 2 * 16)));
    }

    #[test]
    fn the_last_valid_tile_resolves() {
        let ts = loaded();
        assert!(ts.tile_rect(1056).is_some());
    }

    #[test]
    fn an_index_past_the_end_has_no_rect() {
        // This is the tileset/version mismatch signal.
        let ts = loaded();
        assert_eq!(ts.tile_rect(1057), None);
        assert_eq!(ts.tile_rect(99_999), None);
    }

    #[test]
    fn rows_are_exposed() {
        assert_eq!(loaded().rows(), 27);
    }

    #[test]
    fn a_sheet_of_the_wrong_size_is_rejected() {
        let m = manifest();
        let png = png_bytes(m.columns * m.tile_width, 10);
        let err = Tileset::load(m, png).expect_err("a short sheet must be rejected");
        assert!(
            matches!(err, TilesetError::DimensionMismatch { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn a_sheet_with_extra_trailing_rows_is_accepted() {
        // Some generators pad the sheet out to a full rectangle.
        let m = manifest();
        let rows = rows_for(m.tile_count, m.columns);
        let png = png_bytes(m.columns * m.tile_width, (rows + 3) * m.tile_height);
        assert!(Tileset::load(m, png).is_ok());
    }

    #[test]
    fn non_png_data_is_rejected() {
        let err = Tileset::load(manifest(), b"not a png".to_vec())
            .expect_err("garbage must be rejected");
        assert!(matches!(err, TilesetError::Decode(_)), "got {err:?}");
    }

    #[test]
    fn zero_geometry_is_rejected() {
        let m = TilesetManifest {
            tile_width: 0,
            ..manifest()
        };
        let err = Tileset::load(m, png_bytes(16, 16)).expect_err("zero width must be rejected");
        assert!(matches!(err, TilesetError::DegenerateGeometry), "got {err:?}");
    }

    #[test]
    fn data_url_is_a_base64_png() {
        let ts = loaded();
        let url = ts.data_url();
        assert!(
            url.starts_with("data:image/png;base64,"),
            "got {}",
            &url[..url.len().min(40)]
        );
        assert!(url.len() > "data:image/png;base64,".len());
    }

    #[test]
    fn the_manifest_survives_loading() {
        assert_eq!(loaded().manifest().id, "test-16");
    }
}
