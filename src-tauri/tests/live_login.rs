//! End-to-end smoke test against a real dgamelaunch server.
//!
//! Ignored by default: it needs the network and a real game account. The rest
//! of the suite is pure and offline, but the SSH transport and the login state
//! machine only meet each other here, and the interesting failures are the
//! ones that come from what a server actually sends -- dgamelaunch draws its
//! menu with cursor positioning and says nothing at all when it rejects a
//! password, neither of which you would guess from the documentation.
//!
//! ```sh
//! NHTILES_TEST_HOST=nethack.alt.org \
//! NHTILES_TEST_USER=someaccount \
//! NHTILES_TEST_PASS=secret \
//!   cargo test --manifest-path src-tauri/Cargo.toml --test live_login -- --ignored --nocapture
//! ```

use std::time::Duration;

use nethack_tiles_lib::autologin::{AutoLogin, AutoLoginState};
use nethack_tiles_lib::session::SessionEvent;
use nethack_tiles_lib::ssh::{self, SshConfig};
use tokio::sync::mpsc;

/// Runs the login and reports where it got to.
async fn run_login(user: &str, password: &str, host: &str) -> AutoLoginState {
    let config = SshConfig::dgamelaunch(host, 22, "nethack");
    let (tx, mut rx) = mpsc::unbounded_channel();
    let session = ssh::connect(config, tx).await.expect("connect");
    let mut login = AutoLogin::new(user, password);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    while login.wants_output() {
        let event = match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(event)) => event,
            _ => break,
        };
        match event {
            SessionEvent::Data(bytes) => {
                let text: String = bytes.iter().map(|&b| b as char).collect();
                if let Some(reply) = login.observe(&text) {
                    session.write(reply).expect("write");
                }
            }
            SessionEvent::Status(message) => eprintln!("[status] {message}"),
            SessionEvent::Closed { reason } => {
                eprintln!("[closed] {reason:?}");
                break;
            }
        }
    }

    let _ = session.disconnect();
    login.state().clone()
}

fn credentials() -> Option<(String, String, String)> {
    Some((
        std::env::var("NHTILES_TEST_HOST").unwrap_or_else(|_| "nethack.alt.org".into()),
        std::env::var("NHTILES_TEST_USER").ok()?,
        std::env::var("NHTILES_TEST_PASS").ok()?,
    ))
}

#[test]
#[ignore = "needs the network and a real game account"]
fn auto_login_gets_all_the_way_in() {
    let Some((host, user, password)) = credentials() else {
        panic!("set NHTILES_TEST_USER and NHTILES_TEST_PASS");
    };
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let state = runtime.block_on(run_login(&user, &password, &host));
    assert_eq!(state, AutoLoginState::LoggedIn, "login did not complete");
}

#[test]
#[ignore = "needs the network and a real game account"]
fn a_wrong_password_is_reported_rather_than_hanging() {
    let Some((host, user, _)) = credentials() else {
        panic!("set NHTILES_TEST_USER and NHTILES_TEST_PASS");
    };
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let state = runtime.block_on(run_login(&user, "definitely-not-the-password", &host));
    assert!(
        matches!(state, AutoLoginState::Failed(_)),
        "expected a rejection, got {state:?}"
    );
}
