//! Builds a tile sheet PNG + manifest from NetHack's `win/share/*.txt` files.
//!
//! ```text
//! tiles2png --id vanilla-3.6.7-16 --name "Vanilla 16x16 (NetHack 3.6.7)" \
//!           --version v36 --columns 40 --out-dir tiles \
//!           monsters.txt objects.txt other.txt
//! ```
//!
//! The input files must be listed in the order NetHack's `tilemap.c` walks
//! them -- monsters, objects, other -- because tile indices are positional and
//! that order is what the server compiles into `glyph2tile`.

use std::path::PathBuf;
use std::process::ExitCode;

use nethack_tiles_lib::glyph::NetHackVersion;
use nethack_tiles_lib::tileset::TilesetManifest;
use nethack_tiles_lib::tilesrc::{compose_sheet, parse_tile_file};

struct Args {
    id: String,
    name: String,
    version: NetHackVersion,
    columns: u32,
    out_dir: PathBuf,
    inputs: Vec<PathBuf>,
}

fn usage() -> &'static str {
    "usage: tiles2png --id ID --name NAME --version v36|v50 [--columns N] \
     --out-dir DIR <monsters.txt> <objects.txt> <other.txt>"
}

fn parse_args() -> Result<Args, String> {
    let mut id = None;
    let mut name = None;
    let mut version = None;
    let mut columns = 40;
    let mut out_dir = None;
    let mut inputs = Vec::new();

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        let mut value = |flag: &str| argv.next().ok_or(format!("{flag} needs a value"));
        match arg.as_str() {
            "--id" => id = Some(value("--id")?),
            "--name" => name = Some(value("--name")?),
            "--out-dir" => out_dir = Some(PathBuf::from(value("--out-dir")?)),
            "--columns" => {
                columns = value("--columns")?
                    .parse()
                    .map_err(|_| "--columns must be a positive integer".to_string())?
            }
            "--version" => {
                version = Some(match value("--version")?.as_str() {
                    "v36" | "3.6" => NetHackVersion::V36,
                    "v50" | "5.0" | "3.7" => NetHackVersion::V50,
                    other => return Err(format!("unknown --version {other:?}")),
                })
            }
            "-h" | "--help" => return Err(usage().to_string()),
            flag if flag.starts_with("--") => return Err(format!("unknown flag {flag}")),
            path => inputs.push(PathBuf::from(path)),
        }
    }

    if inputs.is_empty() {
        return Err("at least one tile source file is required".into());
    }
    Ok(Args {
        id: id.ok_or("--id is required")?,
        name: name.ok_or("--name is required")?,
        version: version.ok_or("--version is required")?,
        columns,
        out_dir: out_dir.ok_or("--out-dir is required")?,
        inputs,
    })
}

fn run() -> Result<(), String> {
    let args = parse_args()?;

    let mut tiles = Vec::new();
    for path in &args.inputs {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", path.display()))?;
        let parsed =
            parse_tile_file(&text).map_err(|e| format!("parsing {}: {e}", path.display()))?;
        println!("{}: {} tiles", path.display(), parsed.len());
        tiles.extend(parsed);
    }

    let sheet = compose_sheet(&tiles, args.columns).map_err(|e| e.to_string())?;
    let manifest = TilesetManifest {
        id: args.id,
        name: args.name,
        version: args.version,
        tile_width: tiles[0].width,
        tile_height: tiles[0].height,
        columns: args.columns,
        tile_count: tiles.len() as u32,
    };

    std::fs::create_dir_all(&args.out_dir)
        .map_err(|e| format!("creating {}: {e}", args.out_dir.display()))?;
    let png_path = args.out_dir.join(format!("{}.png", manifest.id));
    let manifest_path = args.out_dir.join(format!("{}.json", manifest.id));

    sheet
        .save(&png_path)
        .map_err(|e| format!("writing {}: {e}", png_path.display()))?;
    let json = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    std::fs::write(&manifest_path, json)
        .map_err(|e| format!("writing {}: {e}", manifest_path.display()))?;

    println!(
        "wrote {} ({}x{}, {} tiles, {} columns)\nwrote {}",
        png_path.display(),
        sheet.width(),
        sheet.height(),
        manifest.tile_count,
        manifest.columns,
        manifest_path.display(),
    );
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("tiles2png: {message}");
            ExitCode::FAILURE
        }
    }
}
