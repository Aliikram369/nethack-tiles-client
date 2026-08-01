//! Saved connection profiles.
//!
//! Profiles live in a TOML file under the OS config directory. Passwords
//! never appear in that file -- they go to the OS keychain, keyed by profile
//! id, behind the [`SecretStore`] trait so the store's logic stays testable
//! without touching the real keychain.
//!
//! The stored password is the **dgamelaunch account password**, not an SSH
//! credential: NAO and Hardfought accept an SSH connection as a shared game
//! user and then prompt for the game account inside the terminal.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::glyph::NetHackVersion;

/// Service name under which passwords are filed in the OS keychain.
const KEYCHAIN_SERVICE: &str = "com.ian.nethack-tiles";

fn default_port() -> u16 {
    22
}
fn default_font_family() -> String {
    "Menlo, DejaVu Sans Mono, Consolas, monospace".to_string()
}
fn default_font_size() -> u32 {
    16
}
fn default_scale() -> f32 {
    1.0
}

/// One saved server connection plus its display preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    /// Stable id; also the keychain account key.
    pub id: String,
    /// Display name in the picker.
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    /// The shared SSH user the server publishes, e.g. `nethack`.
    pub ssh_user: String,
    /// The dgamelaunch account name typed at the in-terminal prompt.
    #[serde(default)]
    pub game_user: String,
    /// NetHack release the server runs; selects the glyph flag layout.
    #[serde(default)]
    pub version: NetHackVersion,
    /// Id of the tileset to render with.
    pub tileset_id: String,
    #[serde(default = "default_font_family")]
    pub font_family: String,
    #[serde(default = "default_font_size")]
    pub font_size: u32,
    #[serde(default = "default_scale")]
    pub scale: f32,
    /// Type the game username and password automatically at the dgamelaunch
    /// prompt.
    #[serde(default)]
    pub auto_login: bool,
}

impl Profile {
    /// A new profile with sensible defaults for `name`.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Profile {
            id: id.into(),
            name: name.into(),
            host: String::new(),
            port: default_port(),
            ssh_user: String::new(),
            game_user: String::new(),
            version: NetHackVersion::default(),
            tileset_id: String::new(),
            font_family: default_font_family(),
            font_size: default_font_size(),
            scale: default_scale(),
            auto_login: false,
        }
    }
}

/// The on-disk document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileDocument {
    #[serde(default)]
    pub profiles: Vec<Profile>,
    /// Id of the profile to preselect on launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("reading {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("writing {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not valid profile TOML: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("serializing profiles: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("no profile with id {0:?}")]
    NoSuchProfile(String),
    #[error("could not locate an OS config directory")]
    NoConfigDir,
    #[error("keychain: {0}")]
    Secret(String),
}

/// Somewhere to keep passwords that is not the config file.
pub trait SecretStore: std::fmt::Debug + Send + Sync {
    fn set_password(&self, profile_id: &str, password: &str) -> Result<(), ProfileError>;
    fn get_password(&self, profile_id: &str) -> Result<Option<String>, ProfileError>;
    fn delete_password(&self, profile_id: &str) -> Result<(), ProfileError>;
}

/// The real OS keychain (Keychain / Credential Manager / Secret Service).
#[derive(Debug, Default)]
pub struct KeyringSecrets;

impl KeyringSecrets {
    fn entry(profile_id: &str) -> Result<keyring::Entry, ProfileError> {
        keyring::Entry::new(KEYCHAIN_SERVICE, profile_id)
            .map_err(|e| ProfileError::Secret(e.to_string()))
    }
}

impl SecretStore for KeyringSecrets {
    fn set_password(&self, profile_id: &str, password: &str) -> Result<(), ProfileError> {
        Self::entry(profile_id)?
            .set_password(password)
            .map_err(|e| ProfileError::Secret(e.to_string()))
    }

    fn get_password(&self, profile_id: &str) -> Result<Option<String>, ProfileError> {
        match Self::entry(profile_id)?.get_password() {
            Ok(p) => Ok(Some(p)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(ProfileError::Secret(e.to_string())),
        }
    }

    fn delete_password(&self, profile_id: &str) -> Result<(), ProfileError> {
        match Self::entry(profile_id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(ProfileError::Secret(e.to_string())),
        }
    }
}

/// An in-memory secret store, for tests and for running without a keychain.
#[derive(Debug, Default)]
pub struct MemorySecrets {
    entries: Mutex<HashMap<String, String>>,
}

impl SecretStore for MemorySecrets {
    fn set_password(&self, profile_id: &str, password: &str) -> Result<(), ProfileError> {
        self.entries
            .lock()
            .unwrap()
            .insert(profile_id.to_string(), password.to_string());
        Ok(())
    }

    fn get_password(&self, profile_id: &str) -> Result<Option<String>, ProfileError> {
        Ok(self.entries.lock().unwrap().get(profile_id).cloned())
    }

    fn delete_password(&self, profile_id: &str) -> Result<(), ProfileError> {
        self.entries.lock().unwrap().remove(profile_id);
        Ok(())
    }
}

/// Profiles on disk plus the secret store their passwords live in.
#[derive(Debug)]
pub struct ProfileStore {
    path: PathBuf,
    doc: ProfileDocument,
    secrets: Box<dyn SecretStore>,
}

impl ProfileStore {
    /// The default config file location, `<os config dir>/nethack-tiles/profiles.toml`.
    pub fn default_path() -> Result<PathBuf, ProfileError> {
        let dirs = directories::ProjectDirs::from("com", "ian", "nethack-tiles")
            .ok_or(ProfileError::NoConfigDir)?;
        Ok(dirs.config_dir().join("profiles.toml"))
    }

    /// Loads profiles from `path`, treating a missing file as "no profiles".
    pub fn load(
        path: impl Into<PathBuf>,
        secrets: Box<dyn SecretStore>,
    ) -> Result<Self, ProfileError> {
        let path = path.into();
        let doc = match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|source| ProfileError::Parse {
                path: path.clone(),
                source,
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => ProfileDocument::default(),
            Err(source) => {
                return Err(ProfileError::Read {
                    path: path.clone(),
                    source,
                })
            }
        };
        Ok(ProfileStore {
            path,
            doc,
            secrets,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn profiles(&self) -> &[Profile] {
        &self.doc.profiles
    }

    pub fn last_used(&self) -> Option<&str> {
        self.doc.last_used.as_deref()
    }

    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.doc.profiles.iter().find(|p| p.id == id)
    }

    /// Inserts `profile`, replacing any existing profile with the same id.
    pub fn upsert(&mut self, profile: Profile) -> Result<(), ProfileError> {
        match self.doc.profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile,
            None => self.doc.profiles.push(profile),
        }
        self.save()
    }

    /// Removes a profile and its stored password.
    pub fn remove(&mut self, id: &str) -> Result<(), ProfileError> {
        let before = self.doc.profiles.len();
        self.doc.profiles.retain(|p| p.id != id);
        if self.doc.profiles.len() == before {
            return Err(ProfileError::NoSuchProfile(id.to_string()));
        }
        if self.doc.last_used.as_deref() == Some(id) {
            self.doc.last_used = None;
        }
        self.secrets.delete_password(id)?;
        self.save()
    }

    /// Records `id` as the profile to preselect next launch.
    pub fn set_last_used(&mut self, id: &str) -> Result<(), ProfileError> {
        if self.get(id).is_none() {
            return Err(ProfileError::NoSuchProfile(id.to_string()));
        }
        self.doc.last_used = Some(id.to_string());
        self.save()
    }

    pub fn set_password(&self, id: &str, password: &str) -> Result<(), ProfileError> {
        self.secrets.set_password(id, password)
    }

    pub fn password(&self, id: &str) -> Result<Option<String>, ProfileError> {
        self.secrets.get_password(id)
    }

    /// Writes the document to disk, creating parent directories as needed.
    fn save(&self) -> Result<(), ProfileError> {
        let text = toml::to_string_pretty(&self.doc)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ProfileError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::write(&self.path, text).map_err(|source| ProfileError::Write {
            path: self.path.clone(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        _dir: TempDir,
        path: PathBuf,
    }

    /// Minimal scoped temp directory; avoids a dev-dependency just for this.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "nethack-tiles-test-{tag}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = std::fs::remove_dir_all(&base);
            std::fs::create_dir_all(&base).expect("temp dir");
            TempDir(base)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(tag: &str) -> Fixture {
        let dir = TempDir::new(tag);
        // Deliberately nested: the store must create missing parents.
        let path = dir.0.join("config").join("profiles.toml");
        Fixture { _dir: dir, path }
    }

    fn store(path: &Path) -> ProfileStore {
        ProfileStore::load(path, Box::new(MemorySecrets::default())).expect("load")
    }

    fn sample() -> Profile {
        Profile {
            host: "nethack.alt.org".into(),
            ssh_user: "nethack".into(),
            game_user: "ian".into(),
            tileset_id: "vanilla-3.6.7-16".into(),
            auto_login: true,
            ..Profile::new("nao", "NetHack.alt.org")
        }
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_store() {
        let f = fixture("missing");
        let s = store(&f.path);
        assert!(s.profiles().is_empty());
        assert_eq!(s.last_used(), None);
    }

    #[test]
    fn an_upserted_profile_survives_a_reload() {
        let f = fixture("roundtrip");
        let mut s = store(&f.path);
        s.upsert(sample()).expect("upsert");

        let reloaded = store(&f.path);
        assert_eq!(reloaded.profiles(), &[sample()]);
    }

    #[test]
    fn upsert_replaces_a_profile_with_the_same_id_instead_of_duplicating() {
        let f = fixture("replace");
        let mut s = store(&f.path);
        s.upsert(sample()).unwrap();
        s.upsert(Profile {
            name: "Renamed".into(),
            ..sample()
        })
        .unwrap();

        assert_eq!(s.profiles().len(), 1);
        assert_eq!(s.profiles()[0].name, "Renamed");
    }

    #[test]
    fn upsert_preserves_the_order_of_existing_profiles() {
        let f = fixture("order");
        let mut s = store(&f.path);
        s.upsert(Profile::new("a", "A")).unwrap();
        s.upsert(Profile::new("b", "B")).unwrap();
        s.upsert(Profile::new("a", "A2")).unwrap();

        let ids: Vec<_> = s.profiles().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn the_config_file_never_contains_the_password() {
        let f = fixture("nopassword");
        let mut s = store(&f.path);
        s.upsert(sample()).unwrap();
        s.set_password("nao", "hunter2").unwrap();

        let on_disk = std::fs::read_to_string(&f.path).expect("config file");
        assert!(
            !on_disk.contains("hunter2"),
            "password leaked into the config file:\n{on_disk}"
        );
        // ...but it is still retrievable from the secret store.
        assert_eq!(s.password("nao").unwrap().as_deref(), Some("hunter2"));
    }

    #[test]
    fn removing_a_profile_also_deletes_its_password() {
        let f = fixture("remove");
        let mut s = store(&f.path);
        s.upsert(sample()).unwrap();
        s.set_password("nao", "hunter2").unwrap();

        s.remove("nao").unwrap();

        assert!(s.profiles().is_empty());
        assert_eq!(s.password("nao").unwrap(), None);
        assert!(store(&f.path).profiles().is_empty());
    }

    #[test]
    fn removing_an_unknown_profile_is_an_error() {
        let f = fixture("remove-unknown");
        let mut s = store(&f.path);
        assert!(matches!(
            s.remove("ghost"),
            Err(ProfileError::NoSuchProfile(_))
        ));
    }

    #[test]
    fn removing_a_profile_clears_last_used_if_it_pointed_there() {
        let f = fixture("remove-last-used");
        let mut s = store(&f.path);
        s.upsert(sample()).unwrap();
        s.set_last_used("nao").unwrap();

        s.remove("nao").unwrap();

        assert_eq!(s.last_used(), None);
    }

    #[test]
    fn last_used_survives_a_reload() {
        let f = fixture("last-used");
        let mut s = store(&f.path);
        s.upsert(sample()).unwrap();
        s.set_last_used("nao").unwrap();

        assert_eq!(store(&f.path).last_used(), Some("nao"));
    }

    #[test]
    fn last_used_must_name_an_existing_profile() {
        let f = fixture("last-used-unknown");
        let mut s = store(&f.path);
        assert!(matches!(
            s.set_last_used("ghost"),
            Err(ProfileError::NoSuchProfile(_))
        ));
    }

    #[test]
    fn get_finds_a_profile_by_id() {
        let f = fixture("get");
        let mut s = store(&f.path);
        s.upsert(sample()).unwrap();
        assert_eq!(s.get("nao").map(|p| p.host.as_str()), Some("nethack.alt.org"));
        assert_eq!(s.get("ghost"), None);
    }

    #[test]
    fn malformed_toml_is_reported_with_its_path() {
        let f = fixture("malformed");
        std::fs::create_dir_all(f.path.parent().unwrap()).unwrap();
        std::fs::write(&f.path, "this is not = = toml").unwrap();

        let err = ProfileStore::load(&f.path, Box::new(MemorySecrets::default()))
            .expect_err("malformed TOML must not load silently");
        assert!(matches!(err, ProfileError::Parse { .. }), "got {err:?}");
    }

    #[test]
    fn optional_fields_fall_back_to_defaults_when_absent() {
        let f = fixture("defaults");
        std::fs::create_dir_all(f.path.parent().unwrap()).unwrap();
        std::fs::write(
            &f.path,
            r#"
[[profiles]]
id = "min"
name = "Minimal"
host = "example.org"
sshUser = "nethack"
tilesetId = "vanilla-3.6.7-16"
"#,
        )
        .unwrap();

        let s = store(&f.path);
        let p = s.get("min").expect("profile");
        assert_eq!(p.port, 22);
        assert_eq!(p.font_size, 16);
        assert_eq!(p.scale, 1.0);
        assert!(!p.auto_login);
        assert_eq!(p.version, NetHackVersion::V36);
    }

    #[test]
    fn saving_creates_missing_parent_directories() {
        let f = fixture("mkdir");
        assert!(!f.path.parent().unwrap().exists());
        let mut s = store(&f.path);
        s.upsert(sample()).unwrap();
        assert!(f.path.exists());
    }
}
