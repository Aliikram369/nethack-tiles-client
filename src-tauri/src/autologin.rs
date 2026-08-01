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
    /// Credentials submitted; the client stops driving the session.
    Done,
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

    /// True once the machine will send no further input.
    pub fn is_finished(&self) -> bool {
        matches!(self.state, AutoLoginState::Done | AutoLoginState::Failed(_))
    }

    /// True while the machine still has a reason to look at server output.
    ///
    /// This outlives [`Self::is_finished`]: after the password is submitted we
    /// keep reading for a short while so a rejection is still noticed.
    pub fn wants_output(&self) -> bool {
        match self.state {
            AutoLoginState::Failed(_) => false,
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

        // A rejection can arrive at any point after we start answering.
        if mentions(&self.window, "login failed") || mentions(&self.window, "invalid password") {
            self.state = AutoLoginState::Failed("the server rejected those credentials".into());
            self.window.clear();
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
}

/// Removes ANSI escape sequences so prompt matching sees plain text.
/// dgamelaunch paints its menus with colour and cursor-positioning codes.
fn strip_ansi(input: &str) -> String {
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

/// True if any line looks like the dgamelaunch "login" menu entry, e.g.
/// `l) login` or ` L) Log in`.
fn offers_login_menu(text: &str) -> bool {
    text.lines().any(|line| {
        let l = line.trim().to_ascii_lowercase();
        l.starts_with("l)") && l.contains("log")
    })
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
        assert_eq!(a.state(), &AutoLoginState::Done);
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
