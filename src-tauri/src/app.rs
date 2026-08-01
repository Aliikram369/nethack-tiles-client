//! Tauri state and commands: the seam between the UI and the backend modules.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::mpsc;

use crate::autologin::{AutoLogin, AutoLoginState};
use crate::debuglog::{file_from_env, TileDebugLog};
use crate::glyph::{GlyphFlags, NetHackVersion};
use crate::demux::{Demuxer, StreamItem, TileEvent};
use crate::profiles::{KeyringSecrets, Profile, ProfileStore};
use crate::ssh::{self, SshConfig, SshEvent, SshSession};
use crate::tileset::{Tileset, TilesetManifest};

/// The tilesets shipped with the app, one per supported NetHack line.
///
/// Embedded rather than bundled as Tauri resources so that dev runs and
/// packaged builds resolve them identically. Regenerate with `tiles2png` (see
/// `README.md`).
///
/// There has to be one per version: tile indices are positional, and 5.0 has
/// 1515 tiles where 3.6.7 has 1082, so a 3.6.7 sheet on a 5.0 server draws
/// the wrong picture for nearly every glyph.
const BUNDLED_TILESETS: &[(&str, &[u8])] = &[
    (
        include_str!("../tiles/vanilla-3.6.7-16.json"),
        include_bytes!("../tiles/vanilla-3.6.7-16.png"),
    ),
    (
        include_str!("../tiles/vanilla-5.0.0-16.json"),
        include_bytes!("../tiles/vanilla-5.0.0-16.png"),
    ),
];

/// Event names. Namespaced so they cannot collide with Tauri's own.
pub mod events {
    /// A batch of demultiplexed [`crate::demux::StreamItem`]s.
    pub const STREAM: &str = "nh://stream";
    /// A [`super::StatusPayload`].
    pub const STATUS: &str = "nh://status";
    /// Fired once per session the first time a tile escape code arrives.
    pub const TILEDATA_SEEN: &str = "nh://tiledata-seen";
}

/// Connection state for the status bar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "state", content = "message")]
pub enum StatusPayload {
    Connecting(String),
    Connected(String),
    /// Progress or advisory text; does not change the connected state.
    Info(String),
    Error(String),
    Closed(Option<String>),
}

/// The stream item actually sent to the UI.
///
/// Identical to [`StreamItem`] except that glyph flags arrive already decoded.
/// The `MG_*` bit layout differs between NetHack versions, and keeping that
/// table in exactly one place -- [`crate::glyph`] -- is worth the conversion.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AppStreamItem {
    Text { bytes: String, prints: bool },
    Event { event: AppTileEvent },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AppTileEvent {
    #[serde(rename_all = "camelCase")]
    GlyphStart {
        tile: u32,
        flags: GlyphFlags,
        /// The undecoded bitmask, for diagnostics.
        raw_flags: u32,
    },
    GlyphEnd,
    SelectWindow {
        winid: Option<i64>,
    },
    FrameSync,
    Sound {
        id: Option<i64>,
    },
}

impl AppStreamItem {
    fn from_demux(item: StreamItem, version: NetHackVersion) -> Self {
        match item {
            // Latin-1: each byte becomes the char of the same value, so the
            // frontend can reconstruct the exact bytes.
            StreamItem::Text { bytes, prints } => AppStreamItem::Text {
                bytes: bytes.iter().map(|&b| b as char).collect(),
                prints,
            },
            StreamItem::Event { event } => AppStreamItem::Event {
                event: match event {
                    TileEvent::GlyphStart { tile, flags } => AppTileEvent::GlyphStart {
                        tile,
                        flags: GlyphFlags::decode(flags, version),
                        raw_flags: flags,
                    },
                    TileEvent::GlyphEnd => AppTileEvent::GlyphEnd,
                    TileEvent::SelectWindow { winid } => AppTileEvent::SelectWindow { winid },
                    TileEvent::FrameSync => AppTileEvent::FrameSync,
                    TileEvent::Sound { id } => AppTileEvent::Sound { id },
                },
            },
        }
    }
}

/// A tileset plus its pixels, ready for the overlay canvas.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TilesetPayload {
    pub manifest: TilesetManifest,
    /// `data:image/png;base64,...`
    pub data_url: String,
}

pub struct AppState {
    profiles: Mutex<ProfileStore>,
    /// Loaded tilesets by id; the bundled one is always present.
    tilesets: Mutex<HashMap<String, Tileset>>,
    session: Mutex<Option<SshSession>>,
}

impl AppState {
    pub fn new() -> Result<Self, String> {
        let path = ProfileStore::default_path().map_err(|e| e.to_string())?;
        let mut profiles =
            ProfileStore::load(path, Box::new(KeyringSecrets)).map_err(|e| e.to_string())?;

        let mut tilesets = HashMap::new();
        for (manifest_json, png) in BUNDLED_TILESETS {
            let manifest: TilesetManifest =
                serde_json::from_str(manifest_json).map_err(|e| e.to_string())?;
            let id = manifest.id.clone();
            let tileset = Tileset::load(manifest, png.to_vec())
                .map_err(|e| format!("the bundled tileset {id} is unusable: {e}"))?;
            tilesets.insert(id, tileset);
        }

        // Nobody's first run should start on an empty screen: offer the two
        // public servers, already pointed at a matching tile sheet.
        if profiles.is_first_run() {
            for mut profile in crate::profiles::default_profiles() {
                profile.tileset_id = sheet_for_version(&tilesets, profile.version);
                if let Err(e) = profiles.upsert(profile) {
                    log::warn!("could not write the default profiles: {e}");
                }
            }
        }

        Ok(AppState {
            profiles: Mutex::new(profiles),
            tilesets: Mutex::new(tilesets),
            session: Mutex::new(None),
        })
    }
}

/// Picks the bundled sheet built for `version`. Tile indices are positional
/// and renumbered between NetHack lines, so this pairing is not cosmetic.
fn sheet_for_version(tilesets: &HashMap<String, Tileset>, version: NetHackVersion) -> String {
    tilesets
        .values()
        .map(|t| t.manifest())
        .find(|m| m.version == version)
        .or_else(|| tilesets.values().map(|t| t.manifest()).next())
        .map(|m| m.id.clone())
        .unwrap_or_default()
}

type CmdResult<T> = Result<T, String>;

/// Surfaces a webview failure in the process log, where it can be read.
/// A blank window with the error trapped in the webview console is useless.
#[tauri::command]
pub fn log_frontend_error(message: String) {
    eprintln!("[frontend] {message}");
}

#[tauri::command]
pub fn list_profiles(state: State<'_, AppState>) -> CmdResult<Vec<Profile>> {
    Ok(state.profiles.lock().unwrap().profiles().to_vec())
}

#[tauri::command]
pub fn last_used_profile(state: State<'_, AppState>) -> CmdResult<Option<String>> {
    Ok(state
        .profiles
        .lock()
        .unwrap()
        .last_used()
        .map(str::to_string))
}

/// Saves a profile. `password` is stored in the OS keychain, never in the
/// config file; passing `None` leaves any existing password untouched.
#[tauri::command]
pub fn save_profile(
    state: State<'_, AppState>,
    profile: Profile,
    password: Option<String>,
) -> CmdResult<()> {
    let mut store = state.profiles.lock().unwrap();
    let id = profile.id.clone();
    store.upsert(profile).map_err(|e| e.to_string())?;
    if let Some(password) = password {
        store.set_password(&id, &password).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn delete_profile(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    state
        .profiles
        .lock()
        .unwrap()
        .remove(&id)
        .map_err(|e| e.to_string())
}

/// Whether a password is on file, so the UI can show "saved" without ever
/// reading the secret back into the webview.
#[tauri::command]
pub fn has_saved_password(state: State<'_, AppState>, id: String) -> CmdResult<bool> {
    Ok(state
        .profiles
        .lock()
        .unwrap()
        .password(&id)
        .map_err(|e| e.to_string())?
        .is_some())
}

#[tauri::command]
pub fn list_tilesets(state: State<'_, AppState>) -> CmdResult<Vec<TilesetManifest>> {
    let tilesets = state.tilesets.lock().unwrap();
    let mut manifests: Vec<_> = tilesets.values().map(|t| t.manifest().clone()).collect();
    manifests.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(manifests)
}

#[tauri::command]
pub fn get_tileset(state: State<'_, AppState>, id: String) -> CmdResult<TilesetPayload> {
    let tilesets = state.tilesets.lock().unwrap();
    let tileset = tilesets
        .get(&id)
        .ok_or_else(|| format!("no tileset with id {id:?}"))?;
    Ok(TilesetPayload {
        manifest: tileset.manifest().clone(),
        data_url: tileset.data_url(),
    })
}

/// Loads a user-supplied sheet. The manifest describes its geometry; get it
/// wrong and tiles come out sheared, so the loader validates the dimensions.
#[tauri::command]
pub fn add_custom_tileset(
    state: State<'_, AppState>,
    manifest: TilesetManifest,
    path: String,
) -> CmdResult<TilesetPayload> {
    let png = std::fs::read(&path).map_err(|e| format!("reading {path}: {e}"))?;
    let tileset = Tileset::load(manifest, png).map_err(|e| e.to_string())?;
    let payload = TilesetPayload {
        manifest: tileset.manifest().clone(),
        data_url: tileset.data_url(),
    };
    state
        .tilesets
        .lock()
        .unwrap()
        .insert(tileset.manifest().id.clone(), tileset);
    Ok(payload)
}

/// Parameters the UI controls per connection attempt.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    pub profile_id: String,
    pub cols: u32,
    pub rows: u32,
}

#[tauri::command]
pub async fn ssh_connect(
    app: AppHandle,
    state: State<'_, AppState>,
    request: ConnectRequest,
) -> CmdResult<()> {
    // Gather everything we need under the lock, then release it: the connect
    // below is async and must not hold a std::sync::Mutex across an await.
    let (profile, password) = {
        let store = state.profiles.lock().unwrap();
        let profile = store
            .get(&request.profile_id)
            .ok_or_else(|| format!("no profile with id {:?}", request.profile_id))?
            .clone();
        let password = if profile.auto_login {
            store.password(&profile.id).map_err(|e| e.to_string())?
        } else {
            None
        };
        (profile, password)
    };

    if state.session.lock().unwrap().is_some() {
        return Err("already connected -- disconnect first".into());
    }

    let mut config = SshConfig::dgamelaunch(&profile.host, profile.port, &profile.ssh_user);
    config.cols = request.cols;
    config.rows = request.rows;

    let (tx, rx) = mpsc::unbounded_channel();
    emit_status(
        &app,
        StatusPayload::Connecting(format!("{}:{}", profile.host, profile.port)),
    );

    let session = ssh::connect(config, tx).await.map_err(|e| {
        let message = e.to_string();
        emit_status(&app, StatusPayload::Error(message.clone()));
        message
    })?;

    let autologin = match (profile.auto_login, password) {
        (true, Some(password)) if !profile.game_user.is_empty() => {
            Some(AutoLogin::new(profile.game_user.clone(), password))
        }
        (true, _) => {
            emit_status(
                &app,
                StatusPayload::Info(
                    "Auto-login is on but the profile has no saved username/password".into(),
                ),
            );
            None
        }
        _ => None,
    };

    *state.session.lock().unwrap() = Some(session.clone());
    emit_status(&app, StatusPayload::Connected(profile.host.clone()));

    let tile_count = state
        .tilesets
        .lock()
        .unwrap()
        .get(&profile.tileset_id)
        .map(|t| t.manifest().tile_count)
        .unwrap_or(0);

    tauri::async_runtime::spawn(consume_session(
        app,
        rx,
        session,
        autologin,
        profile.version,
        tile_count,
    ));

    if let Ok(mut store) = state.profiles.lock() {
        let _ = store.set_last_used(&profile.id);
    }
    Ok(())
}

/// Owns the demuxer and auto-login machine for one session's lifetime.
async fn consume_session(
    app: AppHandle,
    mut events: mpsc::UnboundedReceiver<SshEvent>,
    session: SshSession,
    mut autologin: Option<AutoLogin>,
    version: NetHackVersion,
    tile_count: u32,
) {
    let mut demuxer = Demuxer::new();
    let mut announced_tiledata = false;

    // Diagnostics, off unless the environment asks for them. See debuglog.rs.
    let mut debug = file_from_env("NHTILES_LOG").map(|f| TileDebugLog::new(f, tile_count));
    let mut raw = file_from_env("NHTILES_RAW");

    while let Some(event) = events.recv().await {
        match event {
            SshEvent::Data(bytes) => {
                if let Some(raw) = raw.as_mut() {
                    use std::io::Write;
                    let _ = raw.write_all(&bytes);
                    let _ = raw.flush();
                }

                if let Some(login) = autologin.as_mut() {
                    // Only the plain text matters here, and only until the
                    // credentials are in.
                    let text: String = bytes.iter().map(|&b| b as char).collect();
                    if let Some(reply) = login.observe(&text) {
                        let _ = session.write(reply);
                    }
                    // Say which login this is about: the status bar has
                    // already reported the *SSH* connection, and the two are
                    // different accounts entirely.
                    let outcome = match login.state() {
                        AutoLoginState::Failed(reason) => Some(StatusPayload::Error(reason.clone())),
                        AutoLoginState::LoggedIn => Some(StatusPayload::Info(format!(
                            "Logged in to the game server as {}",
                            login.username()
                        ))),
                        _ => None,
                    };
                    if let Some(status) = outcome {
                        emit_status(&app, status);
                        autologin = None;
                    } else if !login.wants_output() {
                        autologin = None;
                    }
                }

                let decoded = demuxer.feed(&bytes);
                if let Some(debug) = debug.as_mut() {
                    debug.observe(&decoded);
                }
                let items: Vec<_> = decoded
                    .into_iter()
                    .map(|item| AppStreamItem::from_demux(item, version))
                    .collect();
                if !items.is_empty() {
                    let _ = app.emit(events::STREAM, &items);
                }
                if !announced_tiledata && demuxer.saw_tile_data() {
                    announced_tiledata = true;
                    let _ = app.emit(events::TILEDATA_SEEN, ());
                }
            }
            SshEvent::Status(message) => emit_status(&app, StatusPayload::Info(message)),
            SshEvent::Closed { reason } => {
                if let Some(debug) = debug.as_mut() {
                    debug.summarize();
                }
                emit_status(&app, StatusPayload::Closed(reason));
                break;
            }
        }
    }

    if let Some(debug) = debug.as_mut() {
        debug.summarize();
    }
    if let Some(state) = app.try_state::<AppState>() {
        *state.session.lock().unwrap() = None;
    }
}

/// Sends keystrokes. `data` is a UTF-8 string from xterm.js `onData`.
#[tauri::command]
pub fn ssh_write(state: State<'_, AppState>, data: String) -> CmdResult<()> {
    with_session(&state, |s| s.write(data.into_bytes()))
}

/// Sends raw bytes.
///
/// NetHack's meta commands are a single byte with the eighth bit set, which
/// no UTF-8 string can carry: encoding U+00EC would put two bytes on the wire
/// and the server would see garbage instead of `M-l`.
#[tauri::command]
pub fn ssh_write_bytes(state: State<'_, AppState>, bytes: Vec<u8>) -> CmdResult<()> {
    with_session(&state, |s| s.write(bytes))
}

#[tauri::command]
pub fn ssh_resize(state: State<'_, AppState>, cols: u32, rows: u32) -> CmdResult<()> {
    with_session(&state, |s| s.resize(cols, rows))
}

#[tauri::command]
pub fn ssh_disconnect(state: State<'_, AppState>) -> CmdResult<()> {
    let session = state.session.lock().unwrap().take();
    match session {
        Some(s) => s.disconnect().map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

fn with_session<T>(
    state: &State<'_, AppState>,
    f: impl FnOnce(&SshSession) -> Result<T, ssh::SshError>,
) -> CmdResult<T> {
    let guard = state.session.lock().unwrap();
    let session = guard.as_ref().ok_or("not connected")?;
    f(session).map_err(|e| e.to_string())
}

fn emit_status(app: &AppHandle, status: StatusPayload) {
    let _ = app.emit(events::STATUS, status);
}

/// Demultiplexed items are emitted as a batch.
pub type StreamBatch = Vec<AppStreamItem>;
