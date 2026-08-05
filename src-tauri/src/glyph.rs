//! Decodes the `MG_*` bitmask carried by `AVTC_GLYPH_START`.
//!
//! The bit values are **version-dependent**. NetHack 5.0 inserted `MG_HERO` at
//! bit 0, shifting everything above it up by one, so a mask decoded with the
//! wrong version turns pets into detected monsters. Compare:
//!
//! | flag         | 3.6.x  | 5.0     |
//! |--------------|--------|---------|
//! | `MG_HERO`    | --     | `0x0001`|
//! | `MG_CORPSE`  | `0x01` | `0x0002`|
//! | `MG_INVIS`   | `0x02` | `0x0004`|
//! | `MG_DETECT`  | `0x04` | `0x0008`|
//! | `MG_PET`     | `0x08` | `0x0010`|
//! | `MG_RIDDEN`  | `0x10` | `0x0020`|
//! | `MG_STATUE`  | `0x20` | `0x0040`|
//! | `MG_OBJPILE` | `0x40` | `0x0080`|
//! | `MG_BW_LAVA` | `0x80` | `0x0100`|
//! | `MG_NOTHING` | --     | `0x0400`|
//! | `MG_UNEXPL`  | --     | `0x0800`|
//! | `MG_FEMALE`  | --     | `0x2000`|
//!
//! Sources: `include/hack.h` (NetHack-3.6.7_Released) and `include/display.h`
//! (master).

use serde::{Deserialize, Serialize};

/// Which NetHack release the server is running, for flag decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum NetHackVersion {
    /// NetHack 3.6.x (3.6.6, 3.6.7 -- what NAO and Hardfought serve today).
    #[default]
    V36,
    /// NetHack 3.7 / 5.0.
    V50,
}

/// The decoded per-glyph flags the overlay can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GlyphFlags {
    /// The glyph is the hero (5.0 only; always false on 3.6).
    pub hero: bool,
    pub corpse: bool,
    pub invisible: bool,
    pub detected: bool,
    /// Draw the pet marker (a heart) over the tile.
    pub pet: bool,
    pub ridden: bool,
    pub statue: bool,
    /// More than one stack of objects on this square.
    pub objpile: bool,
    /// Lava that should be highlighted because it renders like water.
    pub bw_lava: bool,
    /// Nothing is known about this cell: the hero has never seen it (5.0 only).
    ///
    /// 5.0 draws unexplored cells as a real glyph rather than leaving them out,
    /// and its tile is a solid opaque black square. Painting it would cover the
    /// terminal's own background across every unvisited part of the map, so the
    /// overlay leaves these cells alone.
    pub unexplored: bool,
    /// The cell is known to hold nothing worth drawing (5.0 only). Same
    /// treatment as [`Self::unexplored`].
    pub nothing: bool,
    /// The monster or statue is female (5.0 only; always false on 3.6).
    pub female: bool,
}

impl GlyphFlags {
    /// Decodes `raw` using the bit layout of `version`.
    pub fn decode(raw: u32, version: NetHackVersion) -> Self {
        let has = |bit: u32| raw & bit != 0;
        match version {
            NetHackVersion::V36 => GlyphFlags {
                hero: false,
                corpse: has(0x01),
                invisible: has(0x02),
                detected: has(0x04),
                pet: has(0x08),
                ridden: has(0x10),
                statue: has(0x20),
                objpile: has(0x40),
                bw_lava: has(0x80),
                // 3.6 stops at MG_BW_LAVA; it has no marker for either of
                // these, and it never draws an unexplored cell as a glyph.
                unexplored: false,
                nothing: false,
                female: false,
            },
            NetHackVersion::V50 => GlyphFlags {
                hero: has(0x0001),
                corpse: has(0x0002),
                invisible: has(0x0004),
                detected: has(0x0008),
                pet: has(0x0010),
                ridden: has(0x0020),
                statue: has(0x0040),
                objpile: has(0x0080),
                bw_lava: has(0x0100),
                nothing: has(0x0400),
                unexplored: has(0x0800),
                female: has(0x2000),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::NetHackVersion::{V36, V50};
    use super::*;

    #[test]
    fn zero_decodes_to_no_flags() {
        assert_eq!(GlyphFlags::decode(0, V36), GlyphFlags::default());
        assert_eq!(GlyphFlags::decode(0, V50), GlyphFlags::default());
    }

    #[test]
    fn pet_bit_differs_between_versions() {
        // The bug this whole module exists to prevent: 0x08 is MG_PET on 3.6
        // but MG_DETECT on 5.0.
        assert!(GlyphFlags::decode(0x08, V36).pet);
        assert!(!GlyphFlags::decode(0x08, V36).detected);

        assert!(GlyphFlags::decode(0x08, V50).detected);
        assert!(!GlyphFlags::decode(0x08, V50).pet);
        assert!(GlyphFlags::decode(0x10, V50).pet);
    }

    #[test]
    fn decodes_every_v36_flag() {
        assert!(GlyphFlags::decode(0x01, V36).corpse);
        assert!(GlyphFlags::decode(0x02, V36).invisible);
        assert!(GlyphFlags::decode(0x04, V36).detected);
        assert!(GlyphFlags::decode(0x08, V36).pet);
        assert!(GlyphFlags::decode(0x10, V36).ridden);
        assert!(GlyphFlags::decode(0x20, V36).statue);
        assert!(GlyphFlags::decode(0x40, V36).objpile);
        assert!(GlyphFlags::decode(0x80, V36).bw_lava);
    }

    #[test]
    fn decodes_every_v50_flag() {
        assert!(GlyphFlags::decode(0x0001, V50).hero);
        assert!(GlyphFlags::decode(0x0002, V50).corpse);
        assert!(GlyphFlags::decode(0x0004, V50).invisible);
        assert!(GlyphFlags::decode(0x0008, V50).detected);
        assert!(GlyphFlags::decode(0x0010, V50).pet);
        assert!(GlyphFlags::decode(0x0020, V50).ridden);
        assert!(GlyphFlags::decode(0x0040, V50).statue);
        assert!(GlyphFlags::decode(0x0080, V50).objpile);
        assert!(GlyphFlags::decode(0x0100, V50).bw_lava);
        assert!(GlyphFlags::decode(0x2000, V50).female);
    }

    #[test]
    fn v50_reports_the_unexplored_and_nothing_markers() {
        // 5.0 sends a real glyph for every cell the hero has not seen, and its
        // tile is a solid opaque black square. The overlay has to know not to
        // paint it, so these two bits cannot be dropped.
        assert!(GlyphFlags::decode(0x0800, V50).unexplored);
        assert!(GlyphFlags::decode(0x0400, V50).nothing);
        assert!(!GlyphFlags::decode(0x0800, V50).nothing);
        assert!(!GlyphFlags::decode(0x0400, V50).unexplored);
    }

    #[test]
    fn v36_has_no_unexplored_or_nothing_marker() {
        // 3.6 stops at MG_BW_LAVA; those bit positions mean nothing there, so
        // decoding must not invent a marker that suppresses real terrain.
        let all_bits = GlyphFlags::decode(0xffff_ffff, V36);
        assert!(!all_bits.unexplored);
        assert!(!all_bits.nothing);
    }

    #[test]
    fn v36_never_reports_hero_or_female() {
        // Those bits mean other things on 3.6; decoding must not invent them.
        let all_bits = GlyphFlags::decode(0xffff_ffff, V36);
        assert!(!all_bits.hero);
        assert!(!all_bits.female);
    }

    #[test]
    fn combined_flags_decode_together() {
        // A ridden female pet on 5.0: MG_PET | MG_RIDDEN | MG_FEMALE.
        let f = GlyphFlags::decode(0x0010 | 0x0020 | 0x2000, V50);
        assert!(f.pet && f.ridden && f.female);
        assert!(!f.corpse && !f.statue && !f.detected);
    }

    #[test]
    fn unknown_high_bits_are_ignored() {
        assert_eq!(GlyphFlags::decode(1 << 30, V36), GlyphFlags::default());
    }
}
