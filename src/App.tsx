import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { GameTerminal } from "./components/GameTerminal";
import { ProfileForm } from "./components/ProfileForm";
import { TILES, Tile } from "./components/Tile";
import type {
  Profile,
  Status,
  TilesetManifest,
  TilesetPayload,
} from "./lib/protocol";
import {
  deleteProfile,
  getTileset,
  listProfiles,
  listTilesets,
  lastUsedProfile,
  onStatus,
  onTiledataSeen,
  saveProfile,
  sshConnect,
  sshDisconnect,
} from "./lib/tauri";

/** How long to wait for tile codes before suggesting the .nethackrc fix. */
const TILEDATA_GRACE_MS = 40_000;

function newProfile(tilesetId: string): Profile {
  return {
    id: `profile-${Date.now().toString(36)}`,
    name: "",
    host: "",
    port: 22,
    sshUser: "nethack",
    gameUser: "",
    version: "v36",
    tilesetId,
    fontFamily: "Menlo, DejaVu Sans Mono, Consolas, monospace",
    fontSize: 16,
    scale: 1,
    autoLogin: false,
  };
}

type Screen = { kind: "list" } | { kind: "edit"; profile: Profile; isNew: boolean };

export default function App() {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [tilesets, setTilesets] = useState<TilesetManifest[]>([]);
  const [tileset, setTileset] = useState<TilesetPayload | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [screen, setScreen] = useState<Screen>({ kind: "list" });
  const [status, setStatus] = useState<Status | null>(null);
  const [connected, setConnected] = useState<Profile | null>(null);
  const [tilesEnabled, setTilesEnabled] = useState(true);
  const [tiledataHint, setTiledataHint] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const graceTimer = useRef<number | null>(null);

  const selected = useMemo(
    () => profiles.find((p) => p.id === selectedId) ?? null,
    [profiles, selectedId],
  );

  const refreshProfiles = useCallback(async () => {
    const [list, last] = await Promise.all([listProfiles(), lastUsedProfile()]);
    setProfiles(list);
    setSelectedId((current) => current ?? last ?? list[0]?.id ?? null);
  }, []);

  useEffect(() => {
    void (async () => {
      try {
        const sheets = await listTilesets();
        setTilesets(sheets);
        if (sheets[0]) setTileset(await getTileset(sheets[0].id));
        await refreshProfiles();
      } catch (e) {
        setError(String(e));
      }
    })();
  }, [refreshProfiles]);

  // Load the sheet the selected profile asks for.
  useEffect(() => {
    if (!selected) return;
    if (tileset?.manifest.id === selected.tilesetId) return;
    getTileset(selected.tilesetId)
      .then(setTileset)
      .catch(() => {
        /* keep the previous sheet; the picker still shows the mismatch */
      });
  }, [selected, tileset]);

  useEffect(() => {
    const unlisten = onStatus((next) => {
      setStatus(next);
      if (next.state === "error") setError(next.message);
      if (next.state === "closed") {
        setConnected(null);
        setTiledataHint(false);
      }
    });
    return () => {
      void unlisten.then((un) => un());
    };
  }, []);

  useEffect(() => {
    const unlisten = onTiledataSeen(() => {
      setTiledataHint(false);
      if (graceTimer.current) window.clearTimeout(graceTimer.current);
    });
    return () => {
      void unlisten.then((un) => un());
    };
  }, []);

  const connect = async (profile: Profile) => {
    setError(null);
    try {
      await sshConnect(profile.id, 80, 24);
      setConnected(profile);
      if (graceTimer.current) window.clearTimeout(graceTimer.current);
      graceTimer.current = window.setTimeout(
        () => setTiledataHint(true),
        TILEDATA_GRACE_MS,
      );
    } catch (e) {
      setError(String(e));
    }
  };

  const disconnect = async () => {
    await sshDisconnect().catch(() => {});
    setConnected(null);
    setTiledataHint(false);
  };

  const handleSave = async (profile: Profile, password: string | null) => {
    try {
      await saveProfile(profile, password);
      await refreshProfiles();
      setSelectedId(profile.id);
      setScreen({ kind: "list" });
    } catch (e) {
      setError(String(e));
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteProfile(id);
      setSelectedId(null);
      await refreshProfiles();
      setScreen({ kind: "list" });
    } catch (e) {
      setError(String(e));
    }
  };

  if (connected) {
    return (
      <div className="app app--playing">
        <header className="play-bar">
          <span className="play-bar__where">
            <Tile tileset={tileset} index={TILES.openDoor} size={14} />
            {connected.name || connected.host}
          </span>
          <span className="play-bar__status">{statusLine(status)}</span>
          <label className="play-bar__toggle">
            <input
              type="checkbox"
              checked={tilesEnabled}
              onChange={(e) => setTilesEnabled(e.target.checked)}
            />
            Tiles
          </label>
          <button onClick={() => void disconnect()}>Disconnect</button>
        </header>

        {tiledataHint && (
          <p className="banner">
            No tiles yet. Add <code>OPTIONS=vt_tiledata</code> to your{" "}
            <code>.nethackrc</code> on the server, then start a new game.
          </p>
        )}

        <GameTerminal
          profile={connected}
          tileset={tileset}
          tilesEnabled={tilesEnabled}
        />
      </div>
    );
  }

  return (
    <div className="app">
      <header className="masthead">
        <div className="masthead__glyphs" aria-hidden="true">
          {[
            TILES.verticalWall,
            TILES.corridor,
            TILES.littleDog,
            TILES.valkyrie,
            TILES.openDoor,
            TILES.staircaseDown,
            TILES.fountain,
            TILES.horizontalWall,
          ].map((index, i) => (
            <Tile key={i} tileset={tileset} index={index} size={24} />
          ))}
        </div>
        <h1>NetHack Tiles</h1>
        <p className="masthead__sub">
          Play on the public servers, with tiles. Your scores stay on their
          leaderboards.
        </p>
      </header>

      {error && (
        <p className="banner banner--error" role="alert">
          {error}
          <button className="banner__dismiss" onClick={() => setError(null)}>
            Dismiss
          </button>
        </p>
      )}

      {screen.kind === "edit" ? (
        <ProfileForm
          profile={screen.profile}
          tilesets={tilesets}
          onSave={(p, pw) => void handleSave(p, pw)}
          onCancel={() => setScreen({ kind: "list" })}
          onDelete={screen.isNew ? null : (id) => void handleDelete(id)}
        />
      ) : (
        <main className="servers">
          <div className="servers__head">
            <h2>Servers</h2>
            <button
              onClick={() =>
                setScreen({
                  kind: "edit",
                  profile: newProfile(tilesets[0]?.id ?? ""),
                  isNew: true,
                })
              }
            >
              Add server
            </button>
          </div>

          {profiles.length === 0 ? (
            <p className="empty">
              No servers yet. Add <strong>nethack.alt.org</strong> or{" "}
              <strong>hardfought.org</strong> to get started.
            </p>
          ) : (
            <ul className="server-list">
              {profiles.map((profile) => (
                <li key={profile.id}>
                  <button
                    className={`server${profile.id === selectedId ? " server--on" : ""}`}
                    onClick={() => setSelectedId(profile.id)}
                    aria-pressed={profile.id === selectedId}
                  >
                    <Tile
                      tileset={tileset}
                      index={
                        profile.id === selectedId ? TILES.openDoor : TILES.verticalWall
                      }
                      size={20}
                    />
                    <span className="server__name">{profile.name || profile.host}</span>
                    <span className="server__where">
                      {profile.sshUser}@{profile.host}
                      {profile.port !== 22 ? `:${profile.port}` : ""}
                    </span>
                    <span className="server__tag">
                      {profile.version === "v36" ? "3.6" : "5.0"}
                    </span>
                  </button>
                  <button
                    className="server__edit"
                    onClick={() => setScreen({ kind: "edit", profile, isNew: false })}
                  >
                    Edit
                  </button>
                </li>
              ))}
            </ul>
          )}

          <div className="connect-row">
            <span className="connect-row__status">{statusLine(status)}</span>
            <button
              className="primary"
              disabled={!selected}
              onClick={() => selected && void connect(selected)}
            >
              Connect
            </button>
          </div>
        </main>
      )}
    </div>
  );
}

function statusLine(status: Status | null): string {
  if (!status) return "";
  switch (status.state) {
    case "connecting":
      return `Connecting to ${status.message}`;
    case "connected":
      return `Connected to ${status.message}`;
    case "info":
      return status.message;
    case "error":
      return status.message;
    case "closed":
      return status.message ? `Disconnected: ${status.message}` : "Disconnected";
  }
}
