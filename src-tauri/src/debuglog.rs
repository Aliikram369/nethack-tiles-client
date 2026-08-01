//! Optional diagnostic log for tile indices seen on the wire.
//!
//! Enabled with environment variables, so it can be turned on for a single
//! session without a rebuild:
//!
//! ```text
//! NHTILES_LOG=/tmp/tiles.log    # decoded glyphs + a summary
//! NHTILES_RAW=/tmp/tiles.raw    # raw bytes from the server, for replay
//! ```
//!
//! The useful part is the pairing: each tile index is logged next to the
//! character NetHack drew for it. `tile=341 ch='@'` says the server thinks
//! tile 341 is the hero, which is enough to detect an offset between the
//! server's tile ordering and the bundled sheet without guessing.

use std::collections::BTreeMap;
use std::io::Write;

use crate::demux::{StreamItem, TileEvent};

/// Accumulates glyph observations and writes them to `out`.
pub struct TileDebugLog<W: Write> {
    out: W,
    /// Number of tiles in the sheet in use, for the out-of-range count.
    tile_count: u32,
    /// The glyph currently open, with the characters seen so far.
    pending: Option<Pending>,
    total: u64,
    distinct: BTreeMap<u32, u64>,
    out_of_range: BTreeMap<u32, u64>,
    /// Tile index -> the characters it was drawn as.
    samples: BTreeMap<u32, String>,
}

struct Pending {
    tile: u32,
    flags: u32,
    text: String,
}

impl<W: Write> TileDebugLog<W> {
    pub fn new(out: W, tile_count: u32) -> Self {
        TileDebugLog {
            out,
            tile_count,
            pending: None,
            total: 0,
            distinct: BTreeMap::new(),
            out_of_range: BTreeMap::new(),
            samples: BTreeMap::new(),
        }
    }

    /// Records a batch of demultiplexed items.
    pub fn observe(&mut self, items: &[StreamItem]) {
        for item in items {
            match item {
                StreamItem::Text { bytes, .. } => {
                    if let Some(pending) = self.pending.as_mut() {
                        let text: String = bytes.iter().map(|&b| b as char).collect();
                        pending.text.push_str(&printable(&text));
                    }
                }
                StreamItem::Event { event } => match event {
                    TileEvent::GlyphStart { tile, flags } => {
                        // A glyph that never closed: flush it rather than lose it.
                        self.close_glyph();
                        self.pending = Some(Pending {
                            tile: *tile,
                            flags: *flags,
                            text: String::new(),
                        });
                    }
                    TileEvent::GlyphEnd => self.close_glyph(),
                    TileEvent::FrameSync => {
                        self.close_glyph();
                        let _ = writeln!(self.out, "--- frame ---");
                    }
                    _ => {}
                },
            }
        }
        let _ = self.out.flush();
    }

    fn close_glyph(&mut self) {
        let Some(Pending { tile, flags, text }) = self.pending.take() else {
            return;
        };
        self.total += 1;
        *self.distinct.entry(tile).or_insert(0) += 1;
        if tile >= self.tile_count {
            *self.out_of_range.entry(tile).or_insert(0) += 1;
        }
        if !text.is_empty() {
            self.samples.entry(tile).or_insert_with(|| text.clone());
        }

        let range = if tile >= self.tile_count { " OUT-OF-RANGE" } else { "" };
        let _ = writeln!(
            self.out,
            "tile={tile:<5} flags=0x{flags:04x} ch={text:?}{range}"
        );
    }

    /// Writes the accumulated picture of the session.
    pub fn summarize(&mut self) {
        self.close_glyph();
        let min = self.distinct.keys().next().copied();
        let max = self.distinct.keys().next_back().copied();
        let out_of_range_total: u64 = self.out_of_range.values().sum();

        let _ = writeln!(self.out, "\n===== summary =====");
        let _ = writeln!(
            self.out,
            "glyphs={} distinct={} min={:?} max={:?} sheet_tile_count={}",
            self.total,
            self.distinct.len(),
            min,
            max,
            self.tile_count
        );
        let _ = writeln!(
            self.out,
            "out_of_range={out_of_range_total} ({} distinct indices)",
            self.out_of_range.len()
        );

        let _ = writeln!(self.out, "\n-- most frequent indices --");
        let mut by_count: Vec<_> = self.distinct.iter().map(|(t, c)| (*c, *t)).collect();
        by_count.sort_unstable_by(|a, b| b.cmp(a));
        for (count, tile) in by_count.iter().take(25) {
            let sample = self.samples.get(tile).cloned().unwrap_or_default();
            let flag = if *tile >= self.tile_count { " OUT-OF-RANGE" } else { "" };
            let _ = writeln!(self.out, "  tile={tile:<5} seen={count:<6} ch={sample:?}{flag}");
        }
        let _ = self.out.flush();
    }
}

/// Keeps the characters a terminal would actually show, dropping escape
/// sequences and control bytes so the sample is the glyph itself.
fn printable(text: &str) -> String {
    crate::autologin::strip_ansi(text)
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

/// Opens the log named by `var`, if set.
pub fn file_from_env(var: &str) -> Option<std::fs::File> {
    let path = std::env::var_os(var)?;
    match std::fs::File::create(&path) {
        Ok(file) => {
            eprintln!("[debug] writing {var} to {}", path.to_string_lossy());
            Some(file)
        }
        Err(e) => {
            eprintln!("[debug] could not open {}: {e}", path.to_string_lossy());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> StreamItem {
        StreamItem::Text {
            bytes: s.as_bytes().to_vec(),
            prints: true,
        }
    }

    fn glyph(tile: u32, flags: u32) -> StreamItem {
        StreamItem::Event {
            event: TileEvent::GlyphStart { tile, flags },
        }
    }

    const END: StreamItem = StreamItem::Event {
        event: TileEvent::GlyphEnd,
    };

    fn log_to_string(items: &[StreamItem], tile_count: u32, summary: bool) -> String {
        let mut log = TileDebugLog::new(Vec::new(), tile_count);
        log.observe(items);
        if summary {
            log.summarize();
        }
        String::from_utf8(log.out).unwrap()
    }

    #[test]
    fn a_glyph_is_logged_with_the_character_it_was_drawn_as() {
        let out = log_to_string(&[glyph(341, 0), text("@"), END], 1515, false);
        assert!(out.contains("tile=341"), "{out}");
        assert!(out.contains(r#"ch="@""#), "{out}");
    }

    #[test]
    fn colour_codes_around_the_character_are_not_logged_as_the_character() {
        let out = log_to_string(
            &[glyph(12, 0), text("\x1b[1;31md\x1b[0m"), END],
            1515,
            false,
        );
        assert!(out.contains(r#"ch="d""#), "{out}");
    }

    #[test]
    fn flags_are_logged_in_hex() {
        let out = log_to_string(&[glyph(7, 0x10), text("d"), END], 1515, false);
        assert!(out.contains("flags=0x0010"), "{out}");
    }

    #[test]
    fn text_outside_a_glyph_is_not_attributed_to_one() {
        let out = log_to_string(
            &[text("message line"), glyph(5, 0), text("<"), END],
            1515,
            false,
        );
        assert!(out.contains(r#"ch="<""#), "{out}");
        assert!(!out.contains("message"), "{out}");
    }

    #[test]
    fn an_index_past_the_sheet_is_flagged() {
        let out = log_to_string(&[glyph(9000, 0), text("?"), END], 1515, false);
        assert!(out.contains("OUT-OF-RANGE"), "{out}");
    }

    #[test]
    fn an_index_inside_the_sheet_is_not_flagged() {
        let out = log_to_string(&[glyph(100, 0), text("d"), END], 1515, false);
        assert!(!out.contains("OUT-OF-RANGE"), "{out}");
    }

    #[test]
    fn a_glyph_left_open_by_the_next_glyph_is_still_recorded() {
        // NetHack always closes glyphs, but a dropped byte must not swallow
        // every later observation.
        let out = log_to_string(&[glyph(1, 0), text("a"), glyph(2, 0), text("b"), END], 1515, false);
        assert!(out.contains("tile=1"), "{out}");
        assert!(out.contains("tile=2"), "{out}");
    }

    #[test]
    fn the_summary_reports_the_index_range_and_out_of_range_count() {
        let out = log_to_string(
            &[
                glyph(10, 0),
                text("a"),
                END,
                glyph(4000, 0),
                text("b"),
                END,
                glyph(4000, 0),
                text("b"),
                END,
            ],
            1515,
            true,
        );
        assert!(out.contains("glyphs=3"), "{out}");
        assert!(out.contains("distinct=2"), "{out}");
        assert!(out.contains("min=Some(10)"), "{out}");
        assert!(out.contains("max=Some(4000)"), "{out}");
        assert!(out.contains("out_of_range=2"), "{out}");
    }

    #[test]
    fn the_summary_lists_the_most_frequent_indices_with_a_sample_character() {
        let mut items = Vec::new();
        for _ in 0..5 {
            items.extend([glyph(2000, 0), text("?"), END]);
        }
        items.extend([glyph(3, 0), text("@"), END]);
        let out = log_to_string(&items, 1515, true);

        assert!(out.contains("tile=2000"), "{out}");
        assert!(out.contains("seen=5"), "{out}");
        assert!(out.contains(r#"ch="?""#), "{out}");
    }

    #[test]
    fn frame_boundaries_are_marked() {
        let out = log_to_string(
            &[
                glyph(1, 0),
                text("a"),
                END,
                StreamItem::Event {
                    event: TileEvent::FrameSync,
                },
            ],
            1515,
            false,
        );
        assert!(out.contains("--- frame ---"), "{out}");
    }
}
