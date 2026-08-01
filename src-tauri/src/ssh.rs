//! SSH transport to a public NetHack server.
//!
//! Public servers run [dgamelaunch]: every player connects as the *same*
//! shared SSH user (`nethack@nethack.alt.org`) and the real account
//! credentials are typed into the terminal afterwards, which is why the
//! interesting authentication logic lives in [`crate::autologin`] rather than
//! here. This module only has to get an interactive PTY and pump bytes.
//!
//! [dgamelaunch]: https://nethackwiki.com/wiki/Dgamelaunch
//!
//! Testing note: this module talks to a real network and is exercised by a
//! manual smoke test against NAO and Hardfought (see `README.md`). The parts
//! with interesting logic -- escape-code demultiplexing and the login
//! sequence -- are deliberately factored out into pure, unit-tested modules.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::ssh_key;
use russh::{ChannelMsg, Disconnect};
use tokio::sync::mpsc;

use crate::session::{Command, Session, SessionEvent};

/// How to prove who we are to the SSH server.
#[derive(Debug, Clone)]
pub enum SshAuth {
    /// No credential at all. dgamelaunch hosts often allow this for the
    /// shared game user.
    None,
    /// The published password for the shared game user (commonly the same as
    /// the username). This is *not* the player's game password.
    Password(String),
    /// A private key, for users who have one registered.
    Key {
        path: PathBuf,
        passphrase: Option<String>,
    },
}

/// What to do when the server's host key is not already trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HostKeyPolicy {
    /// Refuse unless the key is already in `known_hosts`.
    Strict,
    /// Record an unknown key on first connection, but refuse a *changed* key.
    #[default]
    TrustOnFirstUse,
    /// Accept anything. Never the default -- this disables the only defence
    /// against a man-in-the-middle.
    AcceptAny,
}

#[derive(Debug, Clone)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    /// The shared SSH user, e.g. `nethack`.
    pub user: String,
    /// Authentication methods, tried in order until one succeeds.
    pub auth: Vec<SshAuth>,
    /// `TERM` value requested for the PTY.
    pub term: String,
    pub cols: u32,
    pub rows: u32,
    pub host_key_policy: HostKeyPolicy,
    /// How long to wait for the connection and handshake.
    pub connect_timeout: Duration,
}

impl SshConfig {
    /// A configuration for a typical dgamelaunch server: try an empty
    /// credential first, then the conventional shared password.
    pub fn dgamelaunch(host: impl Into<String>, port: u16, user: impl Into<String>) -> Self {
        let user = user.into();
        SshConfig {
            host: host.into(),
            port,
            auth: vec![SshAuth::None, SshAuth::Password(user.clone())],
            user,
            term: "xterm-256color".into(),
            cols: 80,
            rows: 24,
            host_key_policy: HostKeyPolicy::default(),
            connect_timeout: Duration::from_secs(20),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("could not reach {host}:{port}: {source}")]
    Connect {
        host: String,
        port: u16,
        #[source]
        source: russh::Error,
    },
    #[error("the host key for {host}:{port} has changed since it was trusted -- \
             refusing to connect; remove the stale entry from ~/.ssh/known_hosts if this is expected")]
    HostKeyChanged { host: String, port: u16 },
    #[error("the host key for {host}:{port} is not in ~/.ssh/known_hosts")]
    HostKeyUnknown { host: String, port: u16 },
    #[error("could not check the host key for {host}:{port}: {message}")]
    HostKeyUncheckable {
        host: String,
        port: u16,
        message: String,
    },
    #[error("{host}:{port} did not answer within {seconds}s")]
    ConnectTimeout {
        host: String,
        port: u16,
        seconds: u64,
    },
    #[error("the server rejected every configured authentication method")]
    AuthFailed,
    #[error("could not read the private key {path}: {message}")]
    KeyLoad { path: PathBuf, message: String },
    #[error("ssh error: {0}")]
    Protocol(#[from] russh::Error),
}

/// Why `check_server_key` rejected a key, recorded so `connect` can report
/// something more useful than russh's generic "unknown key".
#[derive(Debug, Clone, PartialEq, Eq)]
enum HostKeyRejection {
    Changed,
    Unknown,
    /// `known_hosts` could not be consulted at all, e.g. it is unreadable.
    /// Worth saying out loud: "not in known_hosts" would send someone looking
    /// for a missing line in a file we never managed to open.
    Uncheckable(String),
}

struct ClientHandler {
    host: String,
    port: u16,
    policy: HostKeyPolicy,
    rejection: Arc<Mutex<Option<HostKeyRejection>>>,
    events: mpsc::UnboundedSender<SessionEvent>,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        if self.policy == HostKeyPolicy::AcceptAny {
            return Ok(true);
        }

        match russh::keys::check_known_hosts(&self.host, self.port, server_public_key) {
            Ok(true) => Ok(true),
            Ok(false) if self.policy == HostKeyPolicy::TrustOnFirstUse => {
                let fingerprint = server_public_key.fingerprint(Default::default());
                if let Err(e) = russh::keys::known_hosts::learn_known_hosts(
                    &self.host,
                    self.port,
                    server_public_key,
                ) {
                    log::warn!("could not record host key in known_hosts: {e}");
                }
                let _ = self.events.send(SessionEvent::Status(format!(
                    "Trusting new host key for {}:{} ({fingerprint})",
                    self.host, self.port
                )));
                Ok(true)
            }
            Ok(false) => {
                *self.rejection.lock().unwrap() = Some(HostKeyRejection::Unknown);
                Ok(false)
            }
            Err(russh::keys::Error::KeyChanged { .. }) => {
                *self.rejection.lock().unwrap() = Some(HostKeyRejection::Changed);
                Ok(false)
            }
            Err(e) => {
                log::warn!("known_hosts lookup failed: {e}");
                *self.rejection.lock().unwrap() =
                    Some(HostKeyRejection::Uncheckable(e.to_string()));
                Ok(false)
            }
        }
    }
}

/// Connects, authenticates, opens an interactive shell, and starts pumping
/// bytes into `events`.
pub async fn connect(
    config: SshConfig,
    events: mpsc::UnboundedSender<SessionEvent>,
) -> Result<Session, SshError> {
    let rejection = Arc::new(Mutex::new(None));
    let handler = ClientHandler {
        host: config.host.clone(),
        port: config.port,
        policy: config.host_key_policy,
        rejection: Arc::clone(&rejection),
        events: events.clone(),
    };

    let russh_config = Arc::new(client::Config {
        // NetHack is turn-based; a player can easily think for minutes.
        inactivity_timeout: Some(Duration::from_secs(3600)),
        keepalive_interval: Some(Duration::from_secs(30)),
        ..Default::default()
    });

    let _ = events.send(SessionEvent::Status(format!(
        "Connecting to {}:{}...",
        config.host, config.port
    )));

    // Bounded, because the failure this guards against looks exactly like a
    // hang: a server that accepts the TCP connection and then never finishes
    // the handshake would otherwise leave the UI saying "Connecting..." for
    // ever, with nothing to act on.
    let attempt = tokio::time::timeout(
        config.connect_timeout,
        client::connect(russh_config, (config.host.clone(), config.port), handler),
    )
    .await
    .map_err(|_| SshError::ConnectTimeout {
        host: config.host.clone(),
        port: config.port,
        seconds: config.connect_timeout.as_secs(),
    })?;

    let mut handle = attempt.map_err(|source| match &*rejection.lock().unwrap() {
        Some(HostKeyRejection::Changed) => SshError::HostKeyChanged {
            host: config.host.clone(),
            port: config.port,
        },
        Some(HostKeyRejection::Unknown) => SshError::HostKeyUnknown {
            host: config.host.clone(),
            port: config.port,
        },
        Some(HostKeyRejection::Uncheckable(message)) => SshError::HostKeyUncheckable {
            host: config.host.clone(),
            port: config.port,
            message: message.clone(),
        },
        None => SshError::Connect {
            host: config.host.clone(),
            port: config.port,
            source,
        },
    })?;

    authenticate(&mut handle, &config).await?;
    // Deliberately explicit: this is the *shared* SSH account, and saying
    // "Authenticated" here reads as though the player's game account is in.
    let _ = events.send(SessionEvent::Status(format!(
        "SSH connected as {} -- game login next",
        config.user
    )));

    let channel = handle.channel_open_session().await?;
    channel
        .request_pty(
            true,
            &config.term,
            config.cols,
            config.rows,
            0,
            0,
            &[],
        )
        .await?;
    channel.request_shell(true).await?;

    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(pump(handle, channel, rx, events));
    Ok(Session::new(tx))
}

/// Tries each configured method until one succeeds.
async fn authenticate(
    handle: &mut Handle<ClientHandler>,
    config: &SshConfig,
) -> Result<(), SshError> {
    for method in &config.auth {
        let succeeded = match method {
            SshAuth::None => handle
                .authenticate_none(config.user.clone())
                .await?
                .success(),
            SshAuth::Password(password) => handle
                .authenticate_password(config.user.clone(), password.clone())
                .await?
                .success(),
            SshAuth::Key { path, passphrase } => {
                let key = russh::keys::load_secret_key(path, passphrase.as_deref()).map_err(
                    |e| SshError::KeyLoad {
                        path: path.clone(),
                        message: e.to_string(),
                    },
                )?;
                let hash = handle.best_supported_rsa_hash().await?.flatten();
                handle
                    .authenticate_publickey(
                        config.user.clone(),
                        russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                    )
                    .await?
                    .success()
            }
        };
        if succeeded {
            return Ok(());
        }

        // Many dgamelaunch hosts only offer keyboard-interactive for what is
        // really password auth, so retry a password that way before moving on.
        if let SshAuth::Password(password) = method {
            if keyboard_interactive(handle, &config.user, password).await? {
                return Ok(());
            }
        }
    }
    Err(SshError::AuthFailed)
}

/// Answers every keyboard-interactive prompt with `password`.
async fn keyboard_interactive(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    password: &str,
) -> Result<bool, SshError> {
    use client::KeyboardInteractiveAuthResponse as Response;

    let mut response = handle
        .authenticate_keyboard_interactive_start(user.to_string(), None)
        .await?;
    loop {
        match response {
            Response::Success => return Ok(true),
            Response::Failure { .. } => return Ok(false),
            Response::InfoRequest { prompts, .. } => {
                let answers = prompts.iter().map(|_| password.to_string()).collect();
                response = handle
                    .authenticate_keyboard_interactive_respond(answers)
                    .await?;
            }
        }
    }
}

/// Owns the channel for the life of the session, forwarding bytes both ways.
async fn pump(
    handle: Handle<ClientHandler>,
    mut channel: russh::Channel<client::Msg>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<SessionEvent>,
) {
    let mut reason = None;
    loop {
        tokio::select! {
            command = commands.recv() => match command {
                Some(Command::Data(bytes)) => {
                    if let Err(e) = channel.data(&bytes[..]).await {
                        reason = Some(format!("send failed: {e}"));
                        break;
                    }
                }
                Some(Command::Resize { cols, rows }) => {
                    if let Err(e) = channel.window_change(cols, rows, 0, 0).await {
                        log::warn!("window_change failed: {e}");
                    }
                }
                Some(Command::Disconnect) | None => break,
            },
            message = channel.wait() => match message {
                Some(ChannelMsg::Data { data }) => {
                    if events.send(SessionEvent::Data(data.to_vec())).is_err() {
                        break; // the UI went away
                    }
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    if events.send(SessionEvent::Data(data.to_vec())).is_err() {
                        break;
                    }
                }
                Some(ChannelMsg::ExitStatus { exit_status }) if exit_status != 0 => {
                    reason = Some(format!("the game exited with status {exit_status}"));
                }
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => break,
                Some(_) => {}
            },
        }
    }

    let _ = handle
        .disconnect(Disconnect::ByApplication, "", "en")
        .await;
    let _ = events.send(SessionEvent::Closed { reason });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dgamelaunch_defaults_try_no_credential_then_the_shared_password() {
        let c = SshConfig::dgamelaunch("nethack.alt.org", 22, "nethack");
        assert_eq!(c.user, "nethack");
        assert!(matches!(c.auth[0], SshAuth::None));
        assert!(matches!(&c.auth[1], SshAuth::Password(p) if p == "nethack"));
    }

    #[test]
    fn the_default_host_key_policy_is_trust_on_first_use_not_accept_any() {
        // Defaulting to AcceptAny would silently disable MITM protection.
        assert_eq!(HostKeyPolicy::default(), HostKeyPolicy::TrustOnFirstUse);
        assert_eq!(
            SshConfig::dgamelaunch("h", 22, "u").host_key_policy,
            HostKeyPolicy::TrustOnFirstUse
        );
    }

    #[test]
    fn connecting_is_bounded_so_a_stalled_handshake_cannot_hang_the_ui() {
        let c = SshConfig::dgamelaunch("h", 22, "u");
        assert!(c.connect_timeout > Duration::ZERO);
        assert!(c.connect_timeout <= Duration::from_secs(60));
    }

    #[test]
    fn an_unreadable_known_hosts_is_not_reported_as_a_missing_entry() {
        // Sending someone to look for a missing line in a file we never
        // managed to open wastes their afternoon.
        let uncheckable = SshError::HostKeyUncheckable {
            host: "nethack.alt.org".into(),
            port: 22,
            message: "Permission denied (os error 1)".into(),
        };
        let rendered = uncheckable.to_string();
        assert!(rendered.contains("could not check"), "{rendered}");
        assert!(rendered.contains("Permission denied"), "{rendered}");
    }
}
