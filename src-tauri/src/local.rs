//! Runs a NetHack installed on this machine, in a pseudo-terminal.
//!
//! NetHack's tty interface talks to a terminal, not a pipe: it asks for the
//! window size with `TIOCGWINSZ`, puts the line discipline in raw mode, and
//! refuses to start at all without a controlling terminal. So a local game
//! needs a real pty, exactly as the SSH transport gets one from the server.
//!
//! Tiles are a *compile-time* option in NetHack (`TTY_TILES_ESCCODES`), and
//! most packaged builds -- Homebrew's included -- are built without it. Such a
//! build plays perfectly well here, in ASCII; it simply never emits a tile
//! code, which is what the "no tiles yet" hint in the UI is for.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::glyph::NetHackVersion;
// `Command` is imported by the pty module below rather than here: only the
// Unix implementation reads from the command channel, and an unused import is
// an error on Windows, where that module does not exist.
use crate::session::{Session, SessionEvent};

/// Directories to look in beyond `PATH`. A packaged NetHack often lands
/// somewhere the app's own inherited `PATH` does not cover -- a GUI app on
/// macOS is not started from a login shell, so it typically sees only
/// `/usr/bin:/bin:/usr/sbin:/sbin`.
const EXTRA_DIRS: &[&str] = &[
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/local/bin",
    "/usr/games",
    "/usr/local/games",
];

/// Executable names a local NetHack is installed under. Deliberately only
/// vanilla: a variant such as NetHack4 or SporkHack numbers its tiles
/// differently, so silently picking one up would draw the wrong map.
const NAMES: &[&str] = &["nethack", "nethack-console"];

#[derive(Debug, thiserror::Error)]
pub enum LocalError {
    #[error("no NetHack found on this machine -- install one (`brew install nethack`) \
             or set the command in the profile")]
    NotFound,
    #[error("{command} is not something this machine can run: {source}")]
    Spawn {
        command: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not open a pseudo-terminal: {0}")]
    Pty(std::io::Error),
    #[error("local play is not supported on this platform yet")]
    Unsupported,
}

/// How to start the local game.
#[derive(Debug, Clone)]
pub struct LocalConfig {
    pub command: PathBuf,
    pub args: Vec<String>,
    /// `TERM` for the child. NetHack looks up its termcap under this name.
    pub term: String,
    pub cols: u16,
    pub rows: u16,
}

impl LocalConfig {
    pub fn new(command: impl Into<PathBuf>) -> Self {
        LocalConfig {
            command: command.into(),
            args: Vec::new(),
            term: "xterm-256color".into(),
            cols: 80,
            rows: 24,
        }
    }
}

/// The directories to search, in priority order: the inherited `PATH` first,
/// then the usual install locations.
pub fn search_dirs(path_env: Option<&str>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = path_env
        .into_iter()
        .flat_map(|p| std::env::split_paths(p).collect::<Vec<_>>())
        .collect();
    for extra in EXTRA_DIRS {
        let extra = PathBuf::from(extra);
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    dirs
}

/// The first NetHack executable in `dirs`, according to `runnable`.
///
/// The predicate is injected so the search order can be tested without
/// depending on what happens to be installed on the machine running the tests.
pub fn find_in(dirs: &[PathBuf], runnable: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    dirs.iter()
        .flat_map(|dir| NAMES.iter().map(move |name| dir.join(name)))
        .find(|candidate| runnable(candidate))
}

/// Looks for a local NetHack on this machine.
pub fn find() -> Option<PathBuf> {
    let path = std::env::var("PATH").ok();
    find_in(&search_dirs(path.as_deref()), is_runnable)
}

#[cfg(unix)]
fn is_runnable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_runnable(path: &Path) -> bool {
    path.is_file()
}

/// Reads the NetHack release out of `nethack --version` output.
///
/// Tile indices are positional and move between releases, so guessing wrong
/// means every picture on the map is wrong. Asking the binary beats assuming.
pub fn parse_version(output: &str) -> Option<NetHackVersion> {
    let at = output.find("Version ")? + "Version ".len();
    let rest = &output[at..];
    let mut parts = rest.split(|c: char| !c.is_ascii_digit());
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    // 3.7 renumbered the tiles and 5.0 continued from there, so anything
    // newer than 3.6 uses the same sheet as 5.0.
    Some(if (major, minor) <= (3, 6) {
        NetHackVersion::V36
    } else {
        NetHackVersion::V50
    })
}

/// How long to let a `--version` probe run. This happens while the window is
/// still being put together, and something on `PATH` called `nethack` that
/// does not answer promptly must not hold the app up.
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Asks a NetHack binary which release it is, falling back to `None` if it
/// will not say -- or will not say quickly.
pub fn probe_version(command: &Path) -> Option<NetHackVersion> {
    let command = command.to_path_buf();
    with_timeout(PROBE_TIMEOUT, move || {
        let output = std::process::Command::new(&command).arg("--version").output().ok()?;
        // Some builds print the banner on stderr.
        parse_version(&String::from_utf8_lossy(&output.stdout))
            .or_else(|| parse_version(&String::from_utf8_lossy(&output.stderr)))
    })
    .flatten()
}

/// Runs `work` on its own thread, giving up on it after `limit`.
///
/// Giving up leaves the thread running: there is no safe way to stop it, and a
/// leaked thread at startup is a far better outcome than a window that never
/// appears.
fn with_timeout<T: Send + 'static>(
    limit: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> Option<T> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(work());
    });
    rx.recv_timeout(limit).ok()
}

/// Starts the game in a pty and pumps bytes into `events`.
#[cfg(unix)]
pub fn spawn(
    config: LocalConfig,
    events: tokio::sync::mpsc::UnboundedSender<SessionEvent>,
) -> Result<Session, LocalError> {
    unix::spawn(config, events)
}

#[cfg(not(unix))]
pub fn spawn(
    _config: LocalConfig,
    _events: tokio::sync::mpsc::UnboundedSender<SessionEvent>,
) -> Result<Session, LocalError> {
    Err(LocalError::Unsupported)
}

#[cfg(unix)]
mod unix {
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::process::Stdio;
    use std::sync::Arc;

    use tokio::sync::mpsc;

    use crate::session::Command;

    use super::{LocalConfig, LocalError, Session, SessionEvent};

    /// One read from the pty. NetHack redraws a screen at a time, so this only
    /// needs to be comfortably larger than a frame to avoid extra syscalls.
    const READ_BUF: usize = 16 * 1024;

    pub fn spawn(
        config: LocalConfig,
        events: mpsc::UnboundedSender<SessionEvent>,
    ) -> Result<Session, LocalError> {
        let (master, slave) = openpty(config.cols, config.rows)?;

        let mut command = std::process::Command::new(&config.command);
        command
            .args(&config.args)
            .env("TERM", &config.term)
            // The child's three standard descriptors are the pty slave, which
            // is what makes it a terminal as far as NetHack is concerned.
            .stdin(Stdio::from(slave.try_clone().map_err(LocalError::Pty)?))
            .stdout(Stdio::from(slave.try_clone().map_err(LocalError::Pty)?))
            .stderr(Stdio::from(slave.try_clone().map_err(LocalError::Pty)?));

        // SAFETY: both calls are async-signal-safe, which is all that is
        // allowed between fork and exec. std runs these closures after it has
        // dup2'd the stdio above into 0/1/2, so fd 0 is the pty slave here.
        unsafe {
            use std::os::unix::process::CommandExt;
            command.pre_exec(|| {
                // A new session, so this process is not in the parent's
                // terminal's process group...
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                // ...and then claim the pty as its controlling terminal, without
                // which NetHack cannot read keys or catch SIGWINCH.
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let mut child = command.spawn().map_err(|source| LocalError::Spawn {
            command: config.command.display().to_string(),
            source,
        })?;
        // Held open only so the child could inherit it; keeping our own copy
        // would mean the read below never sees EOF when the game exits.
        drop(slave);

        let pid = child.id() as libc::pid_t;
        let master = Arc::new(File::from(master));
        let (tx, mut commands) = mpsc::unbounded_channel();
        // Closed by the reader thread when the game exits, which is what stops
        // the writer waiting for keystrokes nobody will type again.
        let (alive, mut ended) = mpsc::unbounded_channel::<()>();

        let _ = events.send(SessionEvent::Status(format!(
            "Playing {} on this machine",
            config.command.display()
        )));

        // A thread, not a task: reading a pty blocks, and blocking a runtime
        // worker for the length of a NetHack game would stall everything else.
        std::thread::spawn({
            let master = Arc::clone(&master);
            move || {
                read_loop(&master, &events);
                // EOF on the master means every copy of the slave is closed,
                // which means the game is gone. Reap it and say why.
                let reason = match child.wait() {
                    Ok(s) if s.success() => None,
                    Ok(s) => Some(format!("NetHack exited with {s}")),
                    Err(e) => Some(format!("could not wait for NetHack: {e}")),
                };
                let _ = events.send(SessionEvent::Closed { reason });
                drop(alive);
            }
        });

        // Writes are short -- a keystroke, or an ioctl -- so a task is fine
        // here, and it can wait on the game ending as well as on the channel.
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    command = commands.recv() => match command {
                        Some(Command::Data(bytes)) => {
                            if (&*master).write_all(&bytes).is_err() {
                                break;
                            }
                        }
                        Some(Command::Resize { cols, rows }) => {
                            set_winsize(master.as_raw_fd(), cols as u16, rows as u16);
                        }
                        Some(Command::Disconnect) | None => {
                            hang_up(pid);
                            break;
                        }
                    },
                    _ = ended.recv() => break,
                }
            }
        });

        Ok(Session::new(tx))
    }

    /// Forwards everything the game writes until it exits.
    fn read_loop(master: &File, events: &mpsc::UnboundedSender<SessionEvent>) {
        let mut buf = vec![0u8; READ_BUF];
        loop {
            match (&*master).read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if events.send(SessionEvent::Data(buf[..n].to_vec())).is_err() {
                        break; // the UI went away
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                // Linux reports the last slave closing as EIO rather than EOF.
                // That is a normal exit, not a failure.
                Err(_) => break,
            }
        }
    }

    /// Ends a local game the way dropping an SSH connection ends a remote one.
    ///
    /// `SIGHUP` and not `SIGKILL`: NetHack handles a hangup by saving the game.
    /// Killing it outright would lose the character and strand a lock file that
    /// blocks the next start.
    fn hang_up(pid: libc::pid_t) {
        // SAFETY: `pid` is our own child, which has not been reaped yet -- the
        // reader thread does that, and it is still waiting on the pty.
        unsafe {
            libc::kill(pid, libc::SIGHUP);
        }
    }

    /// Opens a pty pair sized for the terminal we are showing.
    pub(super) fn openpty(cols: u16, rows: u16) -> Result<(OwnedFd, OwnedFd), LocalError> {
        let size = winsize(cols, rows);
        let mut master: RawFd = -1;
        let mut slave: RawFd = -1;
        // SAFETY: both fds are written by openpty on success; `size` outlives
        // the call.
        let rc = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &size as *const _ as *mut _,
            )
        };
        if rc != 0 {
            return Err(LocalError::Pty(std::io::Error::last_os_error()));
        }
        // SAFETY: openpty just handed us these, and nothing else owns them.
        // Wrapped before anything else can fail, so neither fd leaks.
        let (master, slave) =
            unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };

        // `openpty` hands back plain descriptors, and a plain descriptor is
        // inherited by every later fork+exec -- including the version probe,
        // which runs `nethack --version` while a game may be in progress. One
        // inherited copy of the slave keeps the pty open after the game exits,
        // so the master never reports the end and a finished game looks like a
        // running one. The child that is *supposed* to have the slave still
        // gets it: dup2 onto 0/1/2 clears the flag on the copy it makes.
        set_cloexec(master.as_raw_fd())?;
        set_cloexec(slave.as_raw_fd())?;
        Ok((master, slave))
    }

    /// Marks a descriptor close-on-exec.
    ///
    /// There is still a hair of a window between `openpty` and this call in
    /// which another thread could fork; closing it entirely would need the
    /// allocation itself to be atomic, which `openpty` does not offer.
    fn set_cloexec(fd: RawFd) -> Result<(), LocalError> {
        // SAFETY: `fd` is open and owned by the caller for the whole call.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        if flags == -1 {
            return Err(LocalError::Pty(std::io::Error::last_os_error()));
        }
        if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } == -1 {
            return Err(LocalError::Pty(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    fn winsize(cols: u16, rows: u16) -> libc::winsize {
        libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }
    }

    /// Tells the child its terminal changed size. NetHack redraws on SIGWINCH,
    /// which the kernel sends when this succeeds.
    fn set_winsize(fd: RawFd, cols: u16, rows: u16) {
        let size = winsize(cols, rows);
        // SAFETY: `fd` is our pty master and `size` outlives the call.
        unsafe {
            libc::ioctl(fd, libc::TIOCSWINSZ as _, &size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `PATH` written the way the host writes one.
    ///
    /// Windows separates with `;`, so a literal "/first:/second" is a single
    /// odd directory there rather than two.
    fn path_env(dirs: [&str; 2]) -> String {
        std::env::join_paths(dirs)
            .expect("no separator in these test paths")
            .into_string()
            .expect("test paths are valid unicode")
    }

    #[test]
    fn the_search_starts_with_the_inherited_path() {
        let dirs = search_dirs(Some(&path_env(["/first", "/second"])));
        assert_eq!(dirs[0], PathBuf::from("/first"));
        assert_eq!(dirs[1], PathBuf::from("/second"));
    }

    #[test]
    fn the_usual_install_locations_are_searched_even_when_not_on_path() {
        // A GUI app on macOS inherits a bare PATH, so Homebrew's bin directory
        // has to be looked in explicitly or a local NetHack is never found.
        let dirs = search_dirs(Some("/usr/bin"));
        assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")), "{dirs:?}");
        assert!(dirs.contains(&PathBuf::from("/usr/games")), "{dirs:?}");
    }

    #[test]
    fn a_directory_on_path_is_not_searched_twice() {
        let dirs = search_dirs(Some("/usr/games"));
        let hits = dirs.iter().filter(|d| *d == &PathBuf::from("/usr/games")).count();
        assert_eq!(hits, 1, "{dirs:?}");
    }

    #[test]
    fn a_missing_path_still_searches_the_usual_locations() {
        assert!(!search_dirs(None).is_empty());
    }

    #[test]
    fn the_first_runnable_nethack_in_order_wins() {
        let dirs = vec![PathBuf::from("/a"), PathBuf::from("/b")];
        let found = find_in(&dirs, |p| p == Path::new("/b/nethack"));
        assert_eq!(found, Some(PathBuf::from("/b/nethack")));
    }

    #[test]
    fn nothing_runnable_means_nothing_found() {
        let dirs = vec![PathBuf::from("/a")];
        assert_eq!(find_in(&dirs, |_| false), None);
    }

    #[test]
    fn a_file_that_is_not_executable_is_not_a_nethack() {
        // `nethack` is also the name of a directory in some installs.
        assert!(!is_runnable(Path::new("/")));
    }

    #[test]
    fn version_three_six_maps_to_the_three_six_tile_sheet() {
        let out = "NetHack Version 3.6.7 - last build Wed Feb 16 12:00:00 2023.";
        assert_eq!(parse_version(out), Some(NetHackVersion::V36));
    }

    #[test]
    fn version_five_maps_to_the_five_zero_tile_sheet() {
        let out = "NetHack Version 5.0.0 - last build Mon Jan  1 00:00:00 2026.";
        assert_eq!(parse_version(out), Some(NetHackVersion::V50));
    }

    #[test]
    fn three_seven_shares_the_five_zero_sheet() {
        // 3.7 is where the tile list was renumbered; 5.0 continued from it.
        let out = "NetHack Version 3.7.0-0 - last build Mon Jan  1 00:00:00 2025.";
        assert_eq!(parse_version(out), Some(NetHackVersion::V50));
    }

    #[test]
    fn the_banner_a_real_build_prints_is_understood() {
        // Verbatim from `nethack --version` on a Homebrew install: the
        // platform prefix is what a stricter parser would trip over.
        let out = "MacOSX NetHack Version 3.6.7 - last build Fri Feb 24 12:33:19 2023.";
        assert_eq!(parse_version(out), Some(NetHackVersion::V36));
    }

    #[test]
    fn a_quick_probe_returns_its_answer() {
        assert_eq!(with_timeout(Duration::from_secs(5), || 7), Some(7));
    }

    #[test]
    fn a_probe_that_will_not_finish_gives_up_instead_of_holding_up_startup() {
        let slow = with_timeout(Duration::from_millis(50), || {
            std::thread::sleep(Duration::from_secs(30));
            7
        });
        assert_eq!(slow, None);
    }

    #[test]
    fn output_with_no_version_in_it_is_not_guessed_at() {
        assert_eq!(parse_version("command not found"), None);
        assert_eq!(parse_version(""), None);
    }
}

#[cfg(all(test, unix))]
mod pty_tests {
    use super::*;
    use crate::session::SessionEvent;
    use std::time::Duration;
    use tokio::sync::mpsc;

    /// Collects output until the session closes, so a test never hangs on a
    /// child that misbehaves.
    async fn run(config: LocalConfig) -> (String, Option<String>) {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _session = spawn(config, tx).expect("spawn");

        let mut text = String::new();
        let mut reason = None;
        loop {
            match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
                Ok(Some(SessionEvent::Data(bytes))) => {
                    text.push_str(&String::from_utf8_lossy(&bytes))
                }
                Ok(Some(SessionEvent::Closed { reason: r })) => {
                    reason = r;
                    break;
                }
                Ok(Some(SessionEvent::Status(_))) => {}
                Ok(None) => break,
                Err(_) => panic!("timed out; got so far: {text:?}"),
            }
        }
        (text, reason)
    }

    /// A pty must not survive into a process that has nothing to do with it.
    ///
    /// `openpty` hands back plain descriptors, and a plain descriptor is
    /// inherited by every `fork`+`exec` the process does afterwards. One
    /// leaked copy of the slave is enough to keep the pty open forever: the
    /// game exits, every fd it held closes, and the master still never reports
    /// the end -- so a finished game would look like one still being played.
    #[test]
    fn a_pty_is_not_inherited_by_unrelated_children() {
        use std::io::Read;
        use std::sync::mpsc;

        let (master, slave) = super::unix::openpty(80, 24).expect("openpty");

        // Something else in the process starts a program, as the version probe
        // does. It has no business holding this pty.
        let mut bystander = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("sleep");
        drop(slave);

        // Nothing holds the slave now, so the master must report the end at
        // once. Read on a thread: the failure being tested for is a block that
        // never returns, which would otherwise hang the whole suite.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 64];
            let _ = tx.send(std::fs::File::from(master).read(&mut buf));
        });
        let ended = rx.recv_timeout(Duration::from_secs(5));

        let _ = bystander.kill();
        let _ = bystander.wait();

        match ended {
            Ok(Ok(0)) | Ok(Err(_)) => {}
            Ok(Ok(n)) => panic!("read {n} bytes from a pty nothing has written to"),
            Err(_) => panic!("the pty outlived its slave: a bystander process inherited it"),
        }
    }

    #[tokio::test]
    async fn a_local_command_runs_and_its_output_comes_back() {
        let mut config = LocalConfig::new("/bin/echo");
        config.args = vec!["hello".into()];
        let (text, reason) = run(config).await;

        assert!(text.contains("hello"), "{text:?}");
        assert_eq!(reason, None, "a clean exit needs no explanation");
    }

    #[tokio::test]
    async fn the_child_gets_a_real_terminal_of_the_size_we_asked_for() {
        // `stty size` fails outright without a controlling terminal, so this
        // covers both halves: the pty exists, and NetHack will see 40x100.
        let mut config = LocalConfig::new("/bin/stty");
        config.args = vec!["size".into()];
        config.cols = 100;
        config.rows = 40;
        let (text, _) = run(config).await;

        assert!(text.contains("40 100"), "{text:?}");
    }

    #[tokio::test]
    async fn a_failing_game_says_so_rather_than_closing_silently() {
        let mut config = LocalConfig::new("/bin/sh");
        config.args = vec!["-c".into(), "exit 3".into()];
        let (_, reason) = run(config).await;

        assert!(
            reason.as_deref().is_some_and(|r| r.contains("3")),
            "got {reason:?}"
        );
    }

    #[tokio::test]
    async fn a_command_this_machine_cannot_run_is_an_error_not_a_hang() {
        let (tx, _rx) = mpsc::unbounded_channel();
        let err = spawn(LocalConfig::new("/nonexistent/nethack"), tx)
            .expect_err("must not pretend to have started");
        assert!(matches!(err, LocalError::Spawn { .. }), "{err:?}");
    }

    /// Starts the NetHack actually installed on this machine and checks it
    /// draws something. Ignored by default: it depends on the machine having
    /// one, and it leaves a save file behind.
    #[tokio::test]
    #[ignore = "needs a NetHack installed on this machine"]
    async fn the_installed_nethack_starts_and_draws() {
        let command = super::find().expect("no NetHack installed to test against");
        let (tx, mut rx) = mpsc::unbounded_channel();
        let session = spawn(LocalConfig::new(&command), tx).expect("spawn");

        let mut text = String::new();
        // Keep going until the tty interface paints, which is the first thing
        // that emits an escape sequence.
        while !text.contains('\x1b') {
            match tokio::time::timeout(Duration::from_secs(15), rx.recv()).await {
                Ok(Some(SessionEvent::Data(bytes))) => {
                    text.push_str(&String::from_utf8_lossy(&bytes));
                    // NetHack stops for this when it cannot write its
                    // scoreboard, which is common on a locked-down install.
                    if text.ends_with("Hit return to continue: ") {
                        session.write(b"\n".to_vec()).expect("answer the prompt");
                    }
                }
                Ok(Some(SessionEvent::Closed { reason })) => {
                    panic!("exited early ({reason:?}): {text:?}")
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => panic!("no output in 15s; got {text:?}"),
            }
        }
        eprintln!("{}: {}", command.display(), text.escape_debug());
        let _ = session.disconnect();
    }

    #[tokio::test]
    async fn keystrokes_reach_the_child() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let mut config = LocalConfig::new("/bin/cat");
        config.args = vec![];
        let session = spawn(config, tx).expect("spawn");
        session.write(b"knock\n".to_vec()).expect("write");

        let mut text = String::new();
        while !text.contains("knock") {
            match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
                Ok(Some(SessionEvent::Data(bytes))) => {
                    text.push_str(&String::from_utf8_lossy(&bytes))
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => panic!("timed out; got {text:?}"),
            }
        }
        assert!(text.contains("knock"), "{text:?}");
        let _ = session.disconnect();
    }
}
