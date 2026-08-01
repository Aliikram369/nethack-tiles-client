# NetHack Tiles

A cross-platform desktop client for playing NetHack on the public servers
(nethack.alt.org, Hardfought) with graphical tiles. It connects over SSH, reads
the `vt_tiledata` escape codes the servers already emit, and paints the vanilla
16×16 tileset over the map. Games run on the server, so scores, dumplogs and
ttyrecs are unaffected.

Tauri 2 (OS webview, no bundled browser) + React/TypeScript frontend, Rust
backend.

## Requirements

- Rust (stable) and Node 18+
- A game account on the server you want to play on
- **`OPTIONS=vt_tiledata` in your `.nethackrc` on the server.** Without it the
  server sends plain ASCII and no tiles appear. Edit it through the dgamelaunch
  menu or the server's web editor. The app detects the absence and says so.

## Running

```sh
npm install
npm run tauri dev      # development
npm run tauri build    # packaged app
```

## Tests

```sh
npm test                                         # frontend (vitest)
cargo test --manifest-path src-tauri/Cargo.toml  # backend
```

The parts with real logic are pure and unit tested: the escape-code demuxer,
glyph-flag decoding, tileset geometry, the profile store, the dgamelaunch login
state machine, the tile grid, the stream player and the overlay painter. The
SSH transport itself is covered by manual smoke testing against a live server.

## How tiles work

Servers compile NetHack with `TTY_TILES_ESCCODES`. With `vt_tiledata` on, the
tty port interleaves private escape codes into the stream
(`win/tty/wintty.c`):

| Code | Meaning |
|---|---|
| `ESC [ 1 ; 0 ; n [ ; m ] z` | Start glyph — `n` is `glyph2tile[glyph]`, `m` is the `MG_*` flag mask |
| `ESC [ 1 ; 1 z` | End glyph |
| `ESC [ 1 ; 2 [ ; w ] z` | Select NetHack window `w` |
| `ESC [ 1 ; 3 z` | End of frame; the game is waiting for input |
| `ESC [ 1 ; 4 ; n z` | Sound cue (NetHack 5.0; parsed and ignored) |

Three details drove the design, and all three differ from a naive reading of
the spec:

**Tile placement needs a terminal.** `tty_print_glyph` moves the cursor *before*
emitting the start-glyph code, so the target cell is wherever the cursor sits
once all preceding bytes are processed. Rather than reimplement a terminal
emulator in the backend to track that, the backend emits an *ordered* stream of
text and events, and the frontend asks xterm.js for the cursor inside a
`write()` callback — the exact point at which the terminal has caught up. See
`src/lib/streamPlayer.ts`.

**The window code is a window id, not a window type.** `print_vt_code2(2, window)`
passes a slot index into tty's `wins[]`, not `NHW_MAP`. Tile placement therefore
keys off `GlyphStart` itself, which NetHack only ever emits for the map.

**The flag bits moved in 5.0.** NetHack 5.0 inserted `MG_HERO` at bit 0, shifting
everything above it: `0x08` is `MG_PET` on 3.6 but `MG_DETECT` on 5.0. Flags are
decoded per profile version in `src-tauri/src/glyph.rs`, which is the single
source of truth — the backend sends the frontend decoded booleans, never raw
bits.

Stale tiles are handled without terminal emulation too: the tile grid records
the character NetHack drew in each cell and drops any tile whose cell no longer
holds it, so menus, message lines and screen clears remove tiles by themselves
(`src/lib/tileGrid.ts`).

## Tilesets

The bundled sheet is the vanilla 16×16 tileset built from NetHack 3.6.7's
`win/share/{monsters,objects,other}.txt` — 1082 tiles, 40 columns. It is
embedded in the binary so dev and packaged builds resolve it identically.

**Tile ordering is version-specific.** A sheet built from a different NetHack
release will be off by some offset. Build the sheet from the same version the
server runs; an index the sheet does not cover is drawn as a `?` placeholder
rather than silently skipped.

To regenerate, download the three files from the matching NetHack tag and run:

```sh
npm run tiles -- --id vanilla-3.6.7-16 --name "Vanilla 16x16 (NetHack 3.6.7)" \
  --version v36 --columns 40 --out-dir src-tauri/tiles \
  monsters.txt objects.txt other.txt
```

The input order matters: `tilemap.c` walks monsters, then objects, then other,
and tile indices are positional.

The tile art is from NetHack and is covered by the
[NetHack General Public License](https://github.com/NetHack/NetHack/blob/NetHack-3.6.7_Released/dat/license).

## Credentials

The public servers do not authenticate players over SSH. Everyone connects as a
shared game user (`nethack@nethack.alt.org`) and dgamelaunch then asks for the
game account inside the terminal. So:

- The **SSH user** in a profile is the shared account, usually `nethack`.
- The **game account** username/password is what auto-login types at the
  in-terminal prompt.
- The password is stored in the OS keychain (Keychain / Credential Manager /
  Secret Service), never in the config file. There is a test asserting the
  config file never contains it.

Auto-login answers the dgamelaunch menu, the username prompt and the password
prompt, then stops. It deliberately does not pick a game from the post-login
menu: those menus differ per server and version, and guessing wrong would start
the wrong game.

Host keys are trusted on first use and recorded in `~/.ssh/known_hosts`. A key
that *changes* is a hard failure, not a prompt.

Profiles live in `profiles.toml` under the OS config directory.

## Layout

```
src/                     frontend
  lib/protocol.ts        wire types shared with the backend
  lib/streamPlayer.ts    replays the ordered stream into xterm.js
  lib/tileGrid.ts        which cells show a tile, and when it goes stale
  lib/overlay.ts         canvas painter
  components/            terminal, profile form, tile ornament
src-tauri/src/
  demux.rs               vt_tiledata state machine
  glyph.rs               version-aware MG_* decoding
  tileset.rs             sheet geometry and validation
  tilesrc.rs             NetHack tile-source parser and sheet composer
  profiles.rs            TOML profiles + keychain
  ssh.rs                 russh transport
  autologin.rs           dgamelaunch login state machine
  app.rs                 Tauri commands and events
  bin/tiles2png.rs       tile sheet generator
```

## Not in v1

Watching other players, ttyrec recording, Hardfought variants (xNetHack,
SpliceHack — different tilesets), and sound (`TTY_SOUND_ESCCODES`).
