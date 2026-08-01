//! Types the dgamelaunch login sequence for the user.
//!
//! NAO and Hardfought do not authenticate players over SSH -- everyone
//! connects as a shared game user (`nethack@nethack.alt.org`) and dgamelaunch
//! then asks for the *game account* credentials inside the terminal:
//!
//! ```text
//!  l) login
//!  r) register new user
//!  w) watch games in progress
//!  => l
//! Please enter your username.
//!  => ian
//! Please enter your password.
//! ```
//!
//! This is a pure state machine over the server's output so it can be tested
//! without a network: feed it decoded text, and it hands back the bytes to
//! send. It deliberately stops once the password is submitted rather than also
//! picking a game from the post-login menu -- those menus differ per server
//! and per NetHack version, and guessing wrong would start the wrong game.

/// How far the login sequence has progressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoLoginState {
    /// Waiting for the dgamelaunch menu so we can choose "login".
    AwaitingMenu,
    /// Waiting for the username prompt.
    AwaitingUsername,
    /// Waiting for the password prompt.
    AwaitingPassword,
    /// Credentials submitted; the outcome is not known yet.
    Done,
    /// The server confirmed the login.
    LoggedIn,
    /// The server rejected the credentials.
    Failed(String),
}

/// Caps the amount of recent output kept for prompt matching. Prompts are
/// short, so this only needs to span a chunk boundary or two.
const WINDOW_LIMIT: usize = 4096;

/// How much output to keep scanning for a rejection message after the
/// password is submitted. Without this the machine would stop looking the
/// instant it finished typing and could never report a bad password; scanning
/// forever would instead risk mistaking in-game text for a login failure.
const FAILURE_WATCH_BYTES: usize = 4096;

/// Drives the dgamelaunch login prompts.
///
/// The password is held in memory for the duration of the login only. Note
/// the observation window holds *server* output, which never echoes the
/// password, so the window is safe to log; the password field is not.
pub struct AutoLogin {
    username: String,
    password: String,
    state: AutoLoginState,
    /// Recent server output with escape sequences stripped.
    window: String,
    /// Bytes observed since the password was submitted.
    watched_after_submit: usize,
}

impl std::fmt::Debug for AutoLogin {
    /// Hand-written so the password never reaches a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoLogin")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("state", &self.state)
            .finish()
    }
}

impl AutoLogin {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        AutoLogin {
            username: username.into(),
            password: password.into(),
            state: AutoLoginState::AwaitingMenu,
            window: String::new(),
            watched_after_submit: 0,
        }
    }

    pub fn state(&self) -> &AutoLoginState {
        &self.state
    }

    /// The account name being logged in, for status messages.
    pub fn username(&self) -> &str {
        &self.username
    }

    /// True once the machine will send no further input.
    pub fn is_finished(&self) -> bool {
        matches!(
            self.state,
            AutoLoginState::Done | AutoLoginState::LoggedIn | AutoLoginState::Failed(_)
        )
    }

    /// True while the machine still has a reason to look at server output.
    ///
    /// This outlives [`Self::is_finished`]: after the password is submitted we
    /// keep reading until the outcome is known, so a rejection is noticed.
    pub fn wants_output(&self) -> bool {
        match self.state {
            AutoLoginState::Failed(_) | AutoLoginState::LoggedIn => false,
            AutoLoginState::Done => self.watched_after_submit < FAILURE_WATCH_BYTES,
            _ => true,
        }
    }

    /// Feeds a chunk of server output, returning bytes to send in response.
    pub fn observe(&mut self, chunk: &str) -> Option<Vec<u8>> {
        if !self.wants_output() {
            return None;
        }
        if self.state == AutoLoginState::Done {
            self.watched_after_submit += chunk.len();
        }

        self.window.push_str(&strip_ansi(chunk));
        if self.window.len() > WINDOW_LIMIT {
            // Keep the tail, trimmed to a char boundary.
            let cut = self.window.len() - WINDOW_LIMIT;
            let cut = (cut..self.window.len())
                .find(|&i| self.window.is_char_boundary(i))
                .unwrap_or(self.window.len());
            self.window.drain(..cut);
        }

        if self.state == AutoLoginState::Done {
            self.judge_outcome();
            return None;
        }

        // Some servers do say so outright, and it can arrive at any point.
        if says_rejected(&self.window) {
            self.reject();
            return None;
        }

        let (reply, next) = match self.state {
            AutoLoginState::AwaitingMenu if offers_login_menu(&self.window) => {
                (b"l".to_vec(), AutoLoginState::AwaitingUsername)
            }
            AutoLoginState::AwaitingUsername if mentions(&self.window, "username") => (
                format!("{}\n", self.username).into_bytes(),
                AutoLoginState::AwaitingPassword,
            ),
            AutoLoginState::AwaitingPassword if mentions(&self.window, "password") => (
                format!("{}\n", self.password).into_bytes(),
                AutoLoginState::Done,
            ),
            _ => return None,
        };

        // Drop what we have already acted on so a redraw of the same prompt
        // does not trigger a second reply.
        self.window.clear();
        self.state = next;
        Some(reply)
    }

    /// Decides whether the submitted credentials were accepted.
    ///
    /// dgamelaunch does not necessarily say anything when it rejects a
    /// password -- nethack.alt.org simply redraws the "Not logged in." menu --
    /// so the menu coming back is the rejection, and the account name on the
    /// screen is the confirmation.
    fn judge_outcome(&mut self) {
        if mentions(&self.window, "logged in as") {
            self.state = AutoLoginState::LoggedIn;
            self.window.clear();
        } else if says_rejected(&self.window)
            || mentions(&self.window, "not logged in")
            || offers_login_menu(&self.window)
        {
            self.reject();
        }
    }

    fn reject(&mut self) {
        self.state = AutoLoginState::Failed(
            "the server rejected that game account name or password".into(),
        );
        self.window.clear();
    }
}

/// Removes ANSI escape sequences so prompt matching sees plain text.
/// dgamelaunch paints its menus with colour and cursor-positioning codes.
pub(crate) fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            i += 1;
            if i < bytes.len() && bytes[i] == b'[' {
                i += 1;
                while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                    i += 1;
                }
                i += 1; // the final byte
            } else {
                // ESC + one byte (e.g. charset selection ESC ( B)
                i += 2;
            }
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// True if the screen offers the dgamelaunch "login" entry, e.g. `l) Login`.
///
/// Deliberately *not* line-anchored. dgamelaunch paints its menu by moving the
/// cursor -- `ESC[8;3Hl) Login ESC[9;3Hr) Register new user` -- and emits no
/// newline anywhere on the screen, so once the escape codes are stripped the
/// whole menu is a single line. Requiring `l)` to start a line meant this
/// never matched a real server.
fn offers_login_menu(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.match_indices("l)").any(|(at, _)| {
        // Reject `control)`: the `l` must start the entry, not end a word.
        let standalone = at == 0 || !lower.as_bytes()[at - 1].is_ascii_alphanumeric();
        standalone && lower[at + 2..].trim_start().starts_with("log")
    })
}

/// Rejection wording, for the servers that bother to send any.
fn says_rejected(text: &str) -> bool {
    mentions(text, "login failed") || mentions(text, "invalid password")
}

fn mentions(text: &str, needle: &str) -> bool {
    text.to_ascii_lowercase().contains(needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login() -> AutoLogin {
        AutoLogin::new("ian", "hunter2")
    }

    const MENU: &str = "\
## Welcome to the public server

 l) login
 r) register new user
 w) watch games in progress
 q) quit
 => ";

    /// The real thing, captured from nethack.alt.org. Note it contains no
    /// newlines at all: every entry is placed with `ESC[row;colH`.
    const NAO_MENU: &str = "\x1b[H\x1b[2J\x1b[2d ## \x1b(B\x1b[0;1m\x1b[33mnethack.alt.org - \
http://nethack.alt.org/\x1b[3;2H\x1b[39m\x1b(B\x1b[m##\x1b[4d\x08\x08## Games on this server \
are recorded for in-progress viewing and playback!\x1b[6;3HNot logged in.\x1b[8;3Hl) \
Login\x1b[9;3Hr) Register new user\x1b[10;3Hw) Watch games in progress\x1b[12;3Hs) server \
info\x1b[13;3Hm) MOTD/news (updated: 2026.07.19)\x1b[15;3Hq) Quit\x1b[19;3H=> ";

    /// The screen NAO draws once the password is accepted.
    const NAO_LOGGED_IN: &str = "\x1b[H\x1b[2J\x1b[2d ## \x1b(B\x1b[0;1m\x1b[33mnethack.alt.org \
- http://nethack.alt.org/\x1b[3;2H\x1b[39m\x1b(B\x1b[m##\x1b[4d\x08\x08##\x1b[6d\x08Logged in \
as: \x1b(B\x1b[0;1mstaticoalt\x1b[8;3H\x1b(B\x1b[mc) Change password\x1b[9;3He) Change email \
address\x1b[10;3Hw) Watch games in progress\x1b[13;3Hp) Play NetHack 5.0.0\x1b[22;3H=> ";

    #[test]
    fn strip_ansi_removes_csi_sequences() {
        assert_eq!(strip_ansi("\x1b[1;31mred\x1b[0m text"), "red text");
        assert_eq!(strip_ansi("\x1b[2J\x1b[Hclear"), "clear");
        assert_eq!(strip_ansi("\x1b(Bplain"), "plain");
        assert_eq!(strip_ansi("no escapes"), "no escapes");
    }

    #[test]
    fn starts_out_awaiting_the_menu() {
        assert_eq!(login().state(), &AutoLoginState::AwaitingMenu);
        assert!(!login().is_finished());
    }

    #[test]
    fn ordinary_output_before_the_menu_sends_nothing() {
        let mut a = login();
        assert_eq!(a.observe("connecting...\r\n"), None);
        assert_eq!(a.state(), &AutoLoginState::AwaitingMenu);
    }

    #[test]
    fn the_menu_selects_the_login_option() {
        let mut a = login();
        assert_eq!(a.observe(MENU), Some(b"l".to_vec()));
        assert_eq!(a.state(), &AutoLoginState::AwaitingUsername);
    }

    #[test]
    fn a_coloured_menu_still_matches() {
        let mut a = login();
        let coloured = "\x1b[2J\x1b[H\x1b[1;32m l) login\x1b[0m\r\n r) register\r\n";
        assert_eq!(a.observe(coloured), Some(b"l".to_vec()));
    }

    #[test]
    fn the_username_prompt_sends_the_username_and_a_newline() {
        let mut a = login();
        a.observe(MENU);
        assert_eq!(
            a.observe("Please enter your username.\r\n => "),
            Some(b"ian\n".to_vec())
        );
        assert_eq!(a.state(), &AutoLoginState::AwaitingPassword);
    }

    #[test]
    fn the_password_prompt_sends_the_password_and_finishes() {
        let mut a = login();
        a.observe(MENU);
        a.observe("Please enter your username.\r\n => ");
        assert_eq!(
            a.observe("Please enter your password.\r\n => "),
            Some(b"hunter2\n".to_vec())
        );
        assert_eq!(a.state(), &AutoLoginState::Done);
        assert!(a.is_finished());
    }

    #[test]
    fn a_prompt_split_across_chunks_still_matches() {
        let mut a = login();
        a.observe(MENU);
        assert_eq!(a.observe("Please enter your user"), None);
        assert_eq!(a.observe("name.\r\n => "), Some(b"ian\n".to_vec()));
    }

    #[test]
    fn the_menu_is_answered_only_once() {
        let mut a = login();
        assert_eq!(a.observe(MENU), Some(b"l".to_vec()));
        // The server redraws the menu while it processes the keystroke.
        assert_eq!(a.observe(MENU), None);
    }

    #[test]
    fn output_after_completion_is_ignored() {
        let mut a = login();
        a.observe(MENU);
        a.observe("Please enter your username.\r\n");
        a.observe("Please enter your password.\r\n");
        assert_eq!(a.observe("Please enter your password.\r\n"), None);
        assert_eq!(a.state(), &AutoLoginState::Done);
    }

    #[test]
    fn the_menu_a_real_server_actually_sends_is_recognised() {
        // dgamelaunch draws its menu with cursor positioning and never emits a
        // newline, so anything that matches per line will never fire.
        let mut a = login();
        assert_eq!(a.observe(NAO_MENU), Some(b"l".to_vec()));
    }

    #[test]
    fn a_full_nao_session_runs_end_to_end() {
        let mut a = login();
        let mut sent = Vec::new();
        for chunk in [
            NAO_MENU,
            "\x1b[H\x1b[2J\x1b[2d ## nethack.alt.org\r\x1b[6d Please enter your username. \
             (blank entry aborts)\r\x1b[8d => ",
            "staticoalt\x1b[H\x1b[2J\x1b[2d ## nethack.alt.org\r\x1b[6d Please enter your \
             password.\r\x1b[8d => ",
            NAO_LOGGED_IN,
        ] {
            if let Some(bytes) = a.observe(chunk) {
                sent.push(String::from_utf8(bytes).unwrap());
            }
        }
        assert_eq!(sent, ["l", "ian\n", "hunter2\n"]);
        assert_eq!(a.state(), &AutoLoginState::LoggedIn);
    }

    #[test]
    fn a_confirmed_login_stops_watching_for_a_rejection() {
        // Otherwise in-game text could still trip the failure detector.
        let mut a = login();
        a.observe(NAO_MENU);
        a.observe("Please enter your username.");
        a.observe("Please enter your password.");
        a.observe(NAO_LOGGED_IN);
        assert!(!a.wants_output());
    }

    #[test]
    fn a_bad_password_is_detected_by_the_login_menu_coming_back() {
        // NAO says nothing at all about the failure -- it just redraws the
        // "Not logged in." menu. Watching for the words "login failed" would
        // never notice.
        let mut a = login();
        a.observe(NAO_MENU);
        a.observe("Please enter your username.");
        a.observe("Please enter your password.");
        a.observe(NAO_MENU);
        assert!(
            matches!(a.state(), AutoLoginState::Failed(_)),
            "got {:?}",
            a.state()
        );
    }

    #[test]
    fn the_login_menu_before_the_password_is_submitted_is_not_a_rejection() {
        // The same menu is what starts the sequence; only its reappearance
        // afterwards means anything.
        let mut a = login();
        a.observe(NAO_MENU);
        assert_eq!(a.state(), &AutoLoginState::AwaitingUsername);
    }

    #[test]
    fn a_rejected_password_is_reported_as_failure() {
        let mut a = login();
        a.observe(MENU);
        a.observe("Please enter your username.\r\n");
        a.observe("Please enter your password.\r\n");
        a.observe("Login failed.\r\n");
        assert!(matches!(a.state(), AutoLoginState::Failed(_)));
        assert!(a.is_finished());
    }

    #[test]
    fn failure_watching_stops_once_the_window_is_exhausted() {
        let mut a = login();
        a.observe(MENU);
        a.observe("Please enter your username.\r\n");
        a.observe("Please enter your password.\r\n");
        assert!(a.wants_output(), "should still watch for a rejection");

        a.observe(&"x".repeat(FAILURE_WATCH_BYTES + 1));
        assert!(!a.wants_output());

        // In-game text that happens to say "login failed" much later must not
        // retroactively mark the login as failed.
        a.observe("You read a scroll labelled LOGIN FAILED.\r\n");
        assert_eq!(a.state(), &AutoLoginState::Done);
    }

    #[test]
    fn a_full_session_runs_end_to_end() {
        let mut a = login();
        let mut sent = Vec::new();
        for chunk in [
            "\x1b[2J\x1b[H",
            MENU,
            "l\r\nPlease enter your username.\r\n => ",
            "ian\r\nPlease enter your password.\r\n => ",
            "\r\nLogged in as ian.\r\n",
        ] {
            if let Some(bytes) = a.observe(chunk) {
                sent.push(String::from_utf8(bytes).unwrap());
            }
        }
        assert_eq!(sent, ["l", "ian\n", "hunter2\n"]);
        assert_eq!(a.state(), &AutoLoginState::LoggedIn);
    }

    #[test]
    fn the_observation_window_stays_bounded() {
        let mut a = login();
        for _ in 0..200 {
            a.observe(&"noise ".repeat(100));
        }
        assert!(
            a.window.len() <= WINDOW_LIMIT * 2,
            "window grew to {}",
            a.window.len()
        );
    }

    #[test]
    fn a_long_banner_before_the_menu_does_not_hide_it() {
        let mut a = login();
        a.observe(&"banner line\r\n".repeat(500));
        assert_eq!(a.observe(MENU), Some(b"l".to_vec()));
    }

    #[test]
    fn debug_output_does_not_leak_the_password() {
        let rendered = format!("{:?}", login());
        assert!(!rendered.contains("hunter2"), "{rendered}");
    }
}
