//! Renders the app icon source PNG from the bundled tile sheet.
//!
//! ```text
//! appicon --out ../app-icon.png [--size 1024]
//! ```
//!
//! Feed the result to `npm run tauri icon` to produce the per-platform sizes,
//! `.icns` and `.ico`. Kept as a tool rather than a checked-in drawing so the
//! icon can be regenerated when the tileset changes, and so what it is made of
//! is written down.

use std::path::PathBuf;
use std::process::ExitCode;

use nethack_tiles_lib::icon::{compose, Room, Sheet, Style, WALLS, WIZARD};
use nethack_tiles_lib::tileset::TilesetManifest;

/// The sheet the icon is cut from, embedded so the tool needs no arguments
/// beyond where to write.
const MANIFEST: &str = include_str!("../tiles/vanilla-3.6.7-16.json");
const SHEET: &[u8] = include_bytes!("../tiles/vanilla-3.6.7-16.png");

fn main() -> ExitCode {
    match run() {
        Ok(path) => {
            println!("wrote {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("appicon: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<PathBuf, String> {
    let mut out = PathBuf::from("app-icon.png");
    let mut size: u32 = 1024;
    // Five cells across: a one-cell wall ring with the wizard filling the
    // three cells inside it, which keeps him legible when the icon is small.
    let mut side: u32 = 5;

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "--out" => out = argv.next().ok_or("--out needs a path")?.into(),
            "--side" => {
                side = argv
                    .next()
                    .ok_or("--side needs an odd number of cells, 3 or more")?
                    .parse()
                    .map_err(|e| format!("--side: {e}"))?
            }
            "--size" => {
                size = argv
                    .next()
                    .ok_or("--size needs a number")?
                    .parse()
                    .map_err(|e| format!("--size: {e}"))?
            }
            other => return Err(format!("unexpected argument {other:?}")),
        }
    }

    let manifest: TilesetManifest =
        serde_json::from_str(MANIFEST).map_err(|e| format!("bundled manifest: {e}"))?;
    let pixels = image::load_from_memory(SHEET)
        .map_err(|e| format!("bundled sheet: {e}"))?
        .to_rgba8();

    let sheet = Sheet {
        pixels,
        tile: manifest.tile_width,
        columns: manifest.columns,
    };
    if side < 3 {
        return Err("--side must be at least 3: a room needs walls and an inside".into());
    }
    let room = Room {
        side,
        walls: WALLS,
        center: WIZARD,
    };
    let icon = compose(&sheet, &room, size, &Style::default());
    icon.save(&out).map_err(|e| format!("writing {}: {e}", out.display()))?;
    Ok(out)
}
