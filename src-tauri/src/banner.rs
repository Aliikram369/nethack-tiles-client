//! Reads the NetHack version out of the startup banner.
//!
//! Tile numbers are positional, so a sheet built for the wrong release draws
//! the wrong picture for almost every glyph. The profile records which release
//! the server runs, but one host can serve several: Hardfought's menu offers
//! 3.4.3, 3.6.7 and 5.0.0 behind a single SSH host, and the profile cannot
//! know which one the player picked.
//!
//! The server says so itself. Every tty NetHack prints this before the game
//! starts, from one `Sprintf` in `util/makedefs.c` that has not changed shape
//! between releases:
//!
//! ```text
//!          Version 5.0.0-0 Unix post-release, built Jul 10 2026 22:34:55.
//! ```
//!
//! Reading the number from that line is the only reliable check left. An
//! out-of-range tile index used to be the mismatch signal, but the two sheets
//! now overlap: 3.6.7 addresses 0..1475 once its statue tiles are counted, and
//! a 5.0 server's indices land inside that range. Index 1469 is "unexplored"
//! on 5.0 and "statue of thug" on 3.6.7, and nothing about the number itself
//! says which was meant.

use crate::glyph::NetHackVersion;

/// What the server said it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerVersion {
    /// The version as printed, e.g. `5.0.0-0`. Shown to the player, so it
    /// stays verbatim.
    pub text: String,
    /// The tile ordering this release uses, or `None` for a release this app
    /// has no sheet for.
    pub version: Option<NetHackVersion>,
}

/// The word in front of the number. Capitalised, which is what keeps game
/// messages that happen to use the word from matching.
const MARKER: &str = "Version ";

/// How much text to hold across reads. The banner line is under 80 columns,
/// and a session is megabytes of map redraws, so only a tail is kept.
const TAIL_LIMIT: usize = 512;

/// Watches the terminal text for the startup banner.
#[derive(Debug, Default)]
pub struct VersionWatch {
    tail: String,
    /// Set once the banner is read. The banner is printed once per game, and
    /// re-reporting it on every later read would be noise.
    done: bool,
}

impl VersionWatch {
    pub fn new() -> Self {
        VersionWatch::default()
    }

    /// Records a chunk of terminal output. Returns the version once, on the
    /// read that completes the banner.
    pub fn observe(&mut self, chunk: &str) -> Option<ServerVersion> {
        if self.done {
            return None;
        }

        // The banner is written with a cursor address in front of it, so the
        // escape codes have to come out before the text reads as a line.
        self.tail.push_str(&crate::autologin::strip_ansi(chunk));

        if let Some(found) = find_version(&self.tail) {
            self.done = true;
            self.tail = String::new();
            return Some(found);
        }

        // Keep only what a split banner could still need.
        if self.tail.len() > TAIL_LIMIT {
            let cut = self.tail.len() - TAIL_LIMIT;
            let cut = (cut..self.tail.len())
                .find(|&i| self.tail.is_char_boundary(i))
                .unwrap_or(self.tail.len());
            self.tail.drain(..cut);
        }
        None
    }

    /// Whether the banner is still to come. Once it is not, the caller can
    /// stop decoding text it would only throw away.
    pub fn wants_output(&self) -> bool {
        !self.done
    }

    /// How much text is held waiting for the rest of a line.
    #[cfg(test)]
    fn buffered(&self) -> usize {
        self.tail.len()
    }
}

/// Finds a complete version token after the marker word.
fn find_version(text: &str) -> Option<ServerVersion> {
    let mut from = 0;
    while let Some(offset) = text[from..].find(MARKER) {
        let at = from + offset;
        from = at + MARKER.len();

        // "Version" starts a word here, not ends one, so a message reading
        // "...SubVersion 1.2 " does not count.
        let preceded_by_word = text[..at].chars().next_back().is_some_and(|c| !c.is_whitespace());
        if preceded_by_word {
            continue;
        }

        // The token has to be followed by something before it can be trusted:
        // a read that stops after "Version 5.0" would otherwise be read as
        // 5.0 when the real text is 5.0.0-0.
        let rest = &text[from..];
        let Some(end) = rest.find(|c: char| c.is_whitespace()) else {
            continue;
        };
        let token = &rest[..end];
        if let Some(version) = parse_token(token) {
            return Some(ServerVersion {
                text: token.to_string(),
                version,
            });
        }
    }
    None
}

/// Reads `3.6.7` or `5.0.0-0` and says which tile ordering it uses.
///
/// The outer `Option` is "this is a version at all"; the inner one is "this
/// app has a sheet for it". A release with no sheet is still worth naming,
/// because naming it is what tells the player why the tiles look wrong.
fn parse_token(token: &str) -> Option<Option<NetHackVersion>> {
    let number = token.split('-').next()?;
    let mut parts = number.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    // Any remaining component must be a number too, or this is prose.
    if parts.any(|p| p.parse::<u32>().is_err()) {
        return None;
    }

    Some(match (major, minor) {
        (3, 6) => Some(NetHackVersion::V36),
        // 3.7 is where the tile ordering changed, and 5.0 is that same
        // lineage renamed.
        (3, 7) | (5, 0) => Some(NetHackVersion::V50),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyph::NetHackVersion::{V36, V50};

    /// Captured from us.hardfought.org, playing "v) NetHack 5.0.0-hdf".
    const HARDFOUGHT_5_0: &str = "\x1b[H\x1b[2J\x1b[H\x1b[5;1HNetHack, Copyright 1985-2026\r\
        \x1b[6;1H         By Stichting Mathematisch Centrum and M. Stephenson.\r\
        \x1b[7;1H         Version 5.0.0-0 Unix post-release, built Jul 10 2026 22:34:55.\r\
        \x1b[8;1H         See license for details.\r\x1b[9;1H\x1b[12;1H";

    /// The same line with the version 3.6.7 prints. One `Sprintf` in
    /// `util/makedefs.c` builds it for both releases:
    ///
    /// ```c
    /// Sprintf(outbuf, "         Version %s %s%s, %s %s.",
    ///         version_string(versbuf), PORT_ID, ...,
    ///         date_via_env ? "revised" : "built", &build_date[4]);
    /// ```
    const NAO_3_6_7: &str = "\x1b[7;1H         Version 3.6.7 Unix post-release, \
        built Feb 10 2023 19:15:47.\r";

    #[test]
    fn the_version_hardfought_prints_is_read_as_5_0() {
        let found = VersionWatch::new().observe(HARDFOUGHT_5_0);
        assert_eq!(
            found,
            Some(ServerVersion {
                text: "5.0.0-0".into(),
                version: Some(V50),
            })
        );
    }

    #[test]
    fn the_version_a_3_6_7_server_prints_is_read_as_3_6() {
        let found = VersionWatch::new().observe(NAO_3_6_7);
        assert_eq!(
            found,
            Some(ServerVersion {
                text: "3.6.7".into(),
                version: Some(V36),
            })
        );
    }

    #[test]
    fn three_seven_uses_the_five_zero_tile_ordering() {
        // 3.7 is where the ordering changed; 5.0 is the same lineage.
        let found = VersionWatch::new().observe("         Version 3.7.0 Unix, built x.");
        assert_eq!(found.and_then(|v| v.version), Some(V50));
    }

    #[test]
    fn a_release_with_no_bundled_sheet_is_named_but_not_guessed() {
        // Hardfought still serves 3.4.3 and 1.3d. Guessing a sheet for those
        // would draw the same wrong-picture mess this module exists to catch.
        let found = VersionWatch::new()
            .observe("         Version 3.4.3 Unix post-release, built Jan 1 2020 00:00:00.");
        assert_eq!(
            found,
            Some(ServerVersion {
                text: "3.4.3".into(),
                version: None,
            })
        );
    }

    #[test]
    fn a_banner_split_across_two_reads_is_still_read() {
        // The banner arrives in whatever pieces the network hands over.
        let mut watch = VersionWatch::new();
        assert_eq!(watch.observe("\x1b[7;1H         Vers"), None);
        assert_eq!(
            watch.observe("ion 5.0.0-0 Unix post-release, built Jul 10 2026 22:34:55.\r"),
            Some(ServerVersion {
                text: "5.0.0-0".into(),
                version: Some(V50),
            })
        );
    }

    #[test]
    fn the_version_is_reported_once_not_on_every_read() {
        let mut watch = VersionWatch::new();
        assert!(watch.observe(HARDFOUGHT_5_0).is_some());
        assert_eq!(watch.observe(HARDFOUGHT_5_0), None);
    }

    #[test]
    fn ordinary_game_text_is_not_mistaken_for_a_banner() {
        let mut watch = VersionWatch::new();
        assert_eq!(watch.observe("You see here a version of a scroll."), None);
        assert_eq!(watch.observe("Velkommen statico, the dwarven Valkyrie!"), None);
    }

    #[test]
    fn the_buffer_does_not_grow_without_bound() {
        // A session is megabytes of map redraws. Only the tail can be kept.
        let mut watch = VersionWatch::new();
        for _ in 0..500 {
            watch.observe(&"x".repeat(1000));
        }
        assert!(
            watch.buffered() < 4096,
            "buffered {} bytes",
            watch.buffered()
        );
    }
}
