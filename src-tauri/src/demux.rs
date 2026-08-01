//! Demultiplexes a NetHack tty stream into ordinary terminal bytes and
//! `vt_tiledata` tile events.
//!
//! NetHack compiled with `TTY_TILES_ESCCODES` (as on nethack.alt.org and
//! Hardfought) interleaves private escape sequences into stdout when the
//! `vt_tiledata` option is on. From `win/tty/wintty.c`:
//!
//! ```c
//! #define TILE_ANSI_COMMAND 'z'
//! #define AVTC_GLYPH_START   0
//! #define AVTC_GLYPH_END     1
//! #define AVTC_SELECT_WINDOW 2
//! #define AVTC_INLINE_SYNC   3
//! #define AVTC_SOUND_PLAY    4   /* NetHack 5.0 only */
//!
//! if (c >= 0) {
//!     if (d >= 0) printf("\033[1;%d;%d;%d%c", i, c, d, TILE_ANSI_COMMAND);
//!     else        printf("\033[1;%d;%d%c", i, c, TILE_ANSI_COMMAND);
//! } else          printf("\033[1;%d%c", i, TILE_ANSI_COMMAND);
//! ```
//!
//! So every tile code is `ESC [ 1 ; <sub> [ ; <c> [ ; <d> ] ] z`. Any other
//! CSI sequence -- including ones that also start with a `1` parameter, such
//! as the very common SGR `ESC [ 1 ; 31 m` -- must pass through untouched.
//!
//! The demuxer emits an *ordered* stream so the frontend can replay text and
//! events in exactly the order NetHack produced them. That ordering is what
//! makes tile placement work: `tty_print_glyph` moves the cursor with
//! `tty_curs` *before* emitting `AVTC_GLYPH_START`, so whoever holds the
//! terminal state can resolve the target cell by reading the cursor position
//! once all preceding text has been processed.

use serde::Serialize;

/// A tile-protocol event decoded out of the stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TileEvent {
    /// `ESC [ 1 ; 0 ; <tile> [ ; <flags> ] z` -- the next character written
    /// stands for tile `tile`. `flags` is NetHack's `MG_*` bitmask, whose bit
    /// values are version-dependent (see [`crate::glyph`]).
    GlyphStart { tile: u32, flags: u32 },
    /// `ESC [ 1 ; 1 z` -- end of the glyph opened by `GlyphStart`.
    GlyphEnd,
    /// `ESC [ 1 ; 2 [ ; <winid> ] z` -- subsequent output belongs to NetHack
    /// window `winid`.
    ///
    /// Note this is a *window id* (a slot in tty's `wins[]`), not an `NHW_*`
    /// window type. `tty_nhgetch` also emits this with no parameter at all as
    /// a "force a re-select" kludge, hence the `Option`.
    SelectWindow { winid: Option<i64> },
    /// `ESC [ 1 ; 3 z` -- NetHack has flushed a frame and is waiting for input.
    FrameSync,
    /// `ESC [ 1 ; 4 ; <id> z` -- NetHack 5.0 sound cue. Decoded so it is not
    /// leaked to the terminal; unused in v1.
    Sound { id: Option<i64> },
}

/// One item of the demultiplexed stream, in the order NetHack produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum StreamItem {
    /// Bytes to hand to the terminal emulator verbatim.
    Text {
        #[serde(with = "serde_bytes_as_latin1")]
        bytes: Vec<u8>,
    },
    /// A decoded tile event.
    Event { event: TileEvent },
}

impl StreamItem {
    fn text(bytes: impl Into<Vec<u8>>) -> Self {
        StreamItem::Text {
            bytes: bytes.into(),
        }
    }
}

/// Serializes raw bytes as a latin-1 string so the frontend can rebuild the
/// exact byte sequence. The stream is not valid UTF-8 in general (NetHack can
/// emit IBMgraphics / DECgraphics high bytes), so `String::from_utf8` would be
/// lossy.
mod serde_bytes_as_latin1 {
    use serde::Serializer;

    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let latin1: String = bytes.iter().map(|&b| b as char).collect();
        s.serialize_str(&latin1)
    }
}

/// Longest CSI sequence we will buffer before deciding it is garbage and
/// flushing it as text. Real sequences are far shorter than this.
const MAX_CSI_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    Esc,
    Csi,
}

/// Incremental, chunk-boundary-safe demultiplexer.
#[derive(Debug)]
pub struct Demuxer {
    state: State,
    /// Bytes accumulated after `ESC [`, including the final byte once seen.
    csi: Vec<u8>,
    /// Passthrough bytes not yet flushed as a [`StreamItem::Text`].
    text: Vec<u8>,
    saw_tile_data: bool,
}

impl Default for Demuxer {
    fn default() -> Self {
        Self::new()
    }
}

impl Demuxer {
    pub fn new() -> Self {
        Demuxer {
            state: State::Ground,
            csi: Vec::new(),
            text: Vec::new(),
            saw_tile_data: false,
        }
    }

    /// True once at least one tile escape code has been decoded. Used to warn
    /// the user that `OPTIONS=vt_tiledata` is missing from their `.nethackrc`.
    pub fn saw_tile_data(&self) -> bool {
        self.saw_tile_data
    }

    /// Feeds a chunk of bytes, returning the items decoded from it.
    ///
    /// A sequence split across chunks is held internally and emitted once
    /// complete, so callers may feed arbitrary chunk sizes.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<StreamItem> {
        let mut out = Vec::new();
        for &b in bytes {
            match self.state {
                State::Ground => {
                    if b == 0x1b {
                        self.state = State::Esc;
                    } else {
                        self.text.push(b);
                    }
                }
                State::Esc => match b {
                    b'[' => {
                        self.csi.clear();
                        self.state = State::Csi;
                    }
                    // ESC ESC: the first one is literal, stay armed for the second.
                    0x1b => self.text.push(0x1b),
                    _ => {
                        self.text.push(0x1b);
                        self.text.push(b);
                        self.state = State::Ground;
                    }
                },
                State::Csi => {
                    if self.csi.len() >= MAX_CSI_LEN {
                        // Runaway sequence -- treat the whole thing as text
                        // rather than growing without bound.
                        self.text.push(0x1b);
                        self.text.push(b'[');
                        let csi = std::mem::take(&mut self.csi);
                        self.text.extend_from_slice(&csi);
                        self.text.push(b);
                        self.state = State::Ground;
                        continue;
                    }
                    self.csi.push(b);
                    if is_csi_final(b) {
                        let csi = std::mem::take(&mut self.csi);
                        self.finish_csi(&csi, &mut out);
                        self.state = State::Ground;
                    }
                }
            }
        }
        self.flush_text(&mut out);
        out
    }

    /// Handles a complete CSI sequence: `csi` is everything after `ESC [`,
    /// with the final byte last.
    fn finish_csi(&mut self, csi: &[u8], out: &mut Vec<StreamItem>) {
        let (&final_byte, body) = csi.split_last().expect("csi always has a final byte");
        let params = parse_params(body);

        let is_tile_code = final_byte == b'z' && params.first() == Some(&Some(1));
        if !is_tile_code {
            self.text.push(0x1b);
            self.text.push(b'[');
            self.text.extend_from_slice(csi);
            return;
        }

        self.saw_tile_data = true;
        let param = |i: usize| params.get(i).copied().flatten();
        let event = match param(1) {
            Some(0) => param(2).map(|tile| TileEvent::GlyphStart {
                tile: tile.max(0) as u32,
                flags: param(3).unwrap_or(0).max(0) as u32,
            }),
            Some(1) => Some(TileEvent::GlyphEnd),
            Some(2) => Some(TileEvent::SelectWindow { winid: param(2) }),
            Some(3) => Some(TileEvent::FrameSync),
            Some(4) => Some(TileEvent::Sound { id: param(2) }),
            // Unknown or absent subcode: consume it so it never reaches the
            // terminal, but emit nothing.
            _ => None,
        };
        if let Some(event) = event {
            self.flush_text(out);
            out.push(StreamItem::Event { event });
        }
    }

    fn flush_text(&mut self, out: &mut Vec<StreamItem>) {
        if !self.text.is_empty() {
            out.push(StreamItem::text(std::mem::take(&mut self.text)));
        }
    }
}

/// CSI sequences terminate on a byte in the range `0x40..=0x7E`.
fn is_csi_final(b: u8) -> bool {
    (0x40..=0x7e).contains(&b)
}

/// Splits `;`-separated numeric parameters. An empty parameter is `None`
/// (meaning "use the default"), matching ECMA-48.
fn parse_params(body: &[u8]) -> Vec<Option<i64>> {
    body.split(|&b| b == b';')
        .map(|p| std::str::from_utf8(p).ok().and_then(|s| s.parse().ok()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds one chunk and returns the items, for brevity in tests.
    fn demux(input: &[u8]) -> Vec<StreamItem> {
        Demuxer::new().feed(input)
    }

    fn text(s: &str) -> StreamItem {
        StreamItem::text(s.as_bytes())
    }

    fn event(e: TileEvent) -> StreamItem {
        StreamItem::Event { event: e }
    }

    #[test]
    fn plain_text_passes_through_unchanged() {
        assert_eq!(demux(b"Hello NetHack"), vec![text("Hello NetHack")]);
    }

    #[test]
    fn empty_input_produces_no_items() {
        assert_eq!(demux(b""), vec![]);
    }

    #[test]
    fn glyph_start_carries_tile_index_and_flags() {
        assert_eq!(
            demux(b"\x1b[1;0;344;16z"),
            vec![event(TileEvent::GlyphStart {
                tile: 344,
                flags: 16
            })]
        );
    }

    #[test]
    fn glyph_start_without_flags_defaults_to_zero() {
        assert_eq!(
            demux(b"\x1b[1;0;12z"),
            vec![event(TileEvent::GlyphStart {
                tile: 12,
                flags: 0
            })]
        );
    }

    #[test]
    fn glyph_end_is_decoded() {
        assert_eq!(demux(b"\x1b[1;1z"), vec![event(TileEvent::GlyphEnd)]);
    }

    #[test]
    fn select_window_carries_the_window_id() {
        assert_eq!(
            demux(b"\x1b[1;2;3z"),
            vec![event(TileEvent::SelectWindow { winid: Some(3) })]
        );
    }

    #[test]
    fn select_window_may_omit_the_window_id() {
        // tty_nhgetch emits this form to force the next select to re-transmit.
        assert_eq!(
            demux(b"\x1b[1;2z"),
            vec![event(TileEvent::SelectWindow { winid: None })]
        );
    }

    #[test]
    fn frame_sync_is_decoded() {
        assert_eq!(demux(b"\x1b[1;3z"), vec![event(TileEvent::FrameSync)]);
    }

    #[test]
    fn sound_cue_is_decoded() {
        assert_eq!(
            demux(b"\x1b[1;4;7z"),
            vec![event(TileEvent::Sound { id: Some(7) })]
        );
    }

    #[test]
    fn tile_codes_are_ordered_with_the_surrounding_text() {
        assert_eq!(
            demux(b"before\x1b[1;0;5;0z@\x1b[1;1zafter"),
            vec![
                text("before"),
                event(TileEvent::GlyphStart { tile: 5, flags: 0 }),
                text("@"),
                event(TileEvent::GlyphEnd),
                text("after"),
            ]
        );
    }

    #[test]
    fn adjacent_text_is_coalesced_into_one_item() {
        assert_eq!(demux(b"abc\x1b[2Jdef"), vec![text("abc\x1b[2Jdef")]);
    }

    #[test]
    fn sgr_sequence_starting_with_param_one_is_not_a_tile_code() {
        // ESC[1;31m (bold red) shares the leading `1` parameter with the tile
        // protocol and must pass through untouched.
        assert_eq!(demux(b"\x1b[1;31mred"), vec![text("\x1b[1;31mred")]);
    }

    #[test]
    fn ordinary_csi_sequences_pass_through() {
        for seq in [
            &b"\x1b[2J"[..],
            &b"\x1b[H"[..],
            &b"\x1b[10;20H"[..],
            &b"\x1b[?25l"[..],
            &b"\x1b[m"[..],
        ] {
            assert_eq!(demux(seq), vec![StreamItem::text(seq)], "seq {:?}", seq);
        }
    }

    #[test]
    fn csi_ending_in_z_without_leading_one_passes_through() {
        assert_eq!(demux(b"\x1b[2;0;5z"), vec![text("\x1b[2;0;5z")]);
    }

    #[test]
    fn unknown_tile_subcode_is_consumed_not_leaked() {
        // No event is emitted, so the surrounding text stays a single run.
        assert_eq!(demux(b"a\x1b[1;9;5zb"), vec![text("ab")]);
    }

    #[test]
    fn glyph_start_without_a_tile_index_is_ignored() {
        assert_eq!(demux(b"\x1b[1;0z"), vec![]);
    }

    #[test]
    fn escape_sequence_split_across_chunks_is_reassembled() {
        let mut d = Demuxer::new();
        assert_eq!(d.feed(b"map\x1b[1;0;"), vec![text("map")]);
        assert_eq!(d.feed(b"344;16"), vec![]);
        assert_eq!(
            d.feed(b"z@"),
            vec![
                event(TileEvent::GlyphStart {
                    tile: 344,
                    flags: 16
                }),
                text("@"),
            ]
        );
    }

    #[test]
    fn sequence_split_at_every_possible_offset_yields_the_same_items() {
        let input = b"a\x1b[1;2;3zb\x1b[1;0;7;4z@\x1b[1;1z\x1b[1;3zc";
        let whole = demux(input);
        for split in 1..input.len() {
            let mut d = Demuxer::new();
            let mut items = d.feed(&input[..split]);
            items.extend(d.feed(&input[split..]));
            assert_eq!(coalesce(items), whole, "split at {split}");
        }
    }

    /// Splitting a chunk can split a text run into two items; that is
    /// legitimate, so compare streams with adjacent text runs merged.
    fn coalesce(items: Vec<StreamItem>) -> Vec<StreamItem> {
        let mut out: Vec<StreamItem> = Vec::new();
        for item in items {
            match (out.last_mut(), item) {
                (Some(StreamItem::Text { bytes: prev }), StreamItem::Text { bytes }) => {
                    prev.extend_from_slice(&bytes)
                }
                (_, item) => out.push(item),
            }
        }
        out
    }

    #[test]
    fn lone_escape_followed_by_non_bracket_passes_through() {
        assert_eq!(demux(b"\x1b(Btext"), vec![text("\x1b(Btext")]);
    }

    #[test]
    fn repeated_escapes_pass_through() {
        assert_eq!(demux(b"\x1b\x1b(B"), vec![text("\x1b\x1b(B")]);
    }

    #[test]
    fn high_bytes_survive_the_round_trip() {
        // DECgraphics/IBMgraphics line drawing uses bytes above 0x7f.
        let items = demux(b"\xc4\xb3\xda");
        assert_eq!(items, vec![StreamItem::text(&b"\xc4\xb3\xda"[..])]);
    }

    #[test]
    fn runaway_csi_is_flushed_as_text_rather_than_buffered_forever() {
        let mut input = b"\x1b[".to_vec();
        input.extend(std::iter::repeat(b'1').take(MAX_CSI_LEN + 10));
        let items = demux(&input);
        let total: usize = items
            .iter()
            .map(|i| match i {
                StreamItem::Text { bytes } => bytes.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(total, input.len(), "no bytes should be swallowed");
    }

    #[test]
    fn saw_tile_data_reports_whether_the_server_sent_tile_codes() {
        let mut d = Demuxer::new();
        d.feed(b"plain ascii \x1b[2J");
        assert!(!d.saw_tile_data());
        d.feed(b"\x1b[1;3z");
        assert!(d.saw_tile_data());
    }

    #[test]
    fn a_realistic_map_row_decodes_to_alternating_glyphs_and_text() {
        // Cursor move, then three glyphs, then the frame sync NetHack emits
        // when it starts waiting for a key.
        let input = b"\x1b[1;2;3z\x1b[3;1H\
                      \x1b[1;0;2378;0z-\x1b[1;1z\
                      \x1b[1;0;2379;0z|\x1b[1;1z\
                      \x1b[1;0;337;16z\x1b[1;31md\x1b[0m\x1b[1;1z\
                      \x1b[1;3z";
        assert_eq!(
            demux(input),
            vec![
                event(TileEvent::SelectWindow { winid: Some(3) }),
                text("\x1b[3;1H"),
                event(TileEvent::GlyphStart {
                    tile: 2378,
                    flags: 0
                }),
                text("-"),
                event(TileEvent::GlyphEnd),
                event(TileEvent::GlyphStart {
                    tile: 2379,
                    flags: 0
                }),
                text("|"),
                event(TileEvent::GlyphEnd),
                event(TileEvent::GlyphStart {
                    tile: 337,
                    flags: 16
                }),
                text("\x1b[1;31md\x1b[0m"),
                event(TileEvent::GlyphEnd),
                event(TileEvent::FrameSync),
            ]
        );
    }
}
