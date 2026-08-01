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
- Two lines in your `.nethackrc` **on the server** (edit it through the
  dgamelaunch menu or the server's web editor):

  ```
  OPTIONS=vt_tiledata
  OPTIONS=windowtype:tty
  ```

  Both are required. `vt_tiledata` is implemented in the **tty** window port
  only — `print_vt_code` lives in `win/tty/wintty.c` and nothing in
  `win/curses/` references it — so `windowtype:curses` sends no tile data no
  matter what else is set. The app detects the absence and says so.

  Note NAO uses a separate rc file per NetHack version (`.nethackrc` for 3.6.x,
  `.nh500rc` for 5.0), so make sure you are editing the one for the version you
  actually play.

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
state machine, the tile grid, the stream player and the overlay painter. Where
a test needed to know what a server really sends, the fixture is a verbatim
capture rather than an invention.

The SSH transport only meets the login machine over a network, so that pairing
has its own smoke test, ignored by default:

```sh
NHTILES_TEST_USER=someaccount NHTILES_TEST_PASS=secret \
  cargo test --manifest-path src-tauri/Cargo.toml --test live_login -- --ignored
```

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

**A tile has to go when something writes over its cell — and comparing
characters cannot tell you that.** An unlit map cell is drawn as a space, and
so is the gap between two words of a menu drawn on top of it, so a tile
anchored to its character survives being covered and gets painted over the
menu. The backend therefore splits the stream into printing and non-printing
runs (`prints` on `StreamItem::Text`), which is enough for the frontend to know
exactly which cells each write landed on: a printing run of *n* characters
occupies the *n* cells ending at the cursor once the terminal has processed it.
Any of those cells that is not a glyph's own character is retired
(`src/lib/streamPlayer.ts`, `src/lib/tileGrid.ts`). The recorded character is
kept as a backstop for anything that moves content around behind our back, such
as a scroll or a resize.

Two related details matter for the same reason:

- **A glyph is anchored as soon as its character is on screen**, not at the end
  of the frame. NetHack writes exactly one character between `AVTC_GLYPH_START`
  and `AVTC_GLYPH_END`, so by the next glyph it is there. Reading it later
  records whatever was drawn *over* the cell, which anchors the tile to the very
  thing that should have retired it.
- **The overlay reconciles at the end of every batch, not only on a frame
  sync.** `AVTC_INLINE_SYNC` comes from `tty_nhgetch`, so it stops the moment
  NetHack exits — and dgamelaunch's own menus contain no tile codes at all.
  Waiting for one meant the last frame of the game stayed painted over the
  launcher.

## Tilesets

Two sheets ship with the app, both vanilla 16×16 at 40 columns, built from
`win/share/{monsters,objects,other}.txt` at the matching release tag:

| Tileset | NetHack | Tiles |
|---|---|---|
| `vanilla-3.6.7-16` | 3.6.7 | 1082 |
| `vanilla-5.0.0-16` | 5.0.0 | 1515 |

They are embedded in the binary so dev and packaged builds resolve them
identically.

**Tile ordering is version-specific, and the two lines are nowhere near
compatible** — 5.0 has 433 more tiles and renumbers almost everything. Picking
the wrong one does not fail loudly; it draws the wrong picture for nearly every
glyph. The profile's NetHack version selects a matching sheet automatically,
and the editor warns if you override it into a mismatch. An index the chosen
sheet does not cover is drawn as a `?` placeholder rather than silently
skipped.

To build a sheet for another version or variant, download the three files from
the matching NetHack tag and run:

```sh
npm run tiles -- --id vanilla-3.6.7-16 --name "Vanilla 16x16 (NetHack 3.6.7)" \
  --version v36 --columns 40 --out-dir src-tauri/tiles \
  monsters.txt objects.txt other.txt
```

The input order matters: `tilemap.c` walks monsters, then objects, then other,
and tile indices are positional.

The tile art is from NetHack and is covered by the
[NetHack General Public License](https://github.com/NetHack/NetHack/blob/NetHack-3.6.7_Released/dat/license).

## Debugging tiles

Two environment variables turn on diagnostics for a session, no rebuild needed:

```sh
NHTILES_LOG=/tmp/tiles.log NHTILES_RAW=/tmp/tiles.raw npm run tauri dev
```

`NHTILES_LOG` records every glyph next to the character NetHack drew for it
(`tile=93 flags=0x0000 ch="@"`) plus a summary of the index range and anything
outside the sheet. That pairing is what identifies an ordering mismatch: if the
hero is `ch="@"` at tile 93 and the sheet's 93 is a rock mole, the sheet is
built for the wrong NetHack version. `NHTILES_RAW` dumps the raw server bytes
for offline replay.

## Display

Tiles are drawn into terminal cells, so the cell *is* the tile — and a
monospace cell is about half as wide as it is tall, which squashes a square
16×16 tile. The **Display** panel in the game bar adjusts font, font size, cell
width (letter spacing), cell height (line height) and whole-pixel tile drawing
while the game is running, and writes the result to the server's profile. The
terminal is re-measured in place rather than rebuilt, so nothing on screen is
lost.

"Whole-pixel tiles" draws each tile at 1×, 2× or 3× its native 16px art,
centred in the cell, instead of stretching it. It needs a cell at least 16px in
both directions, which is what the size and cell-width controls are for; below
that it falls back to stretching, since a native-size tile would spill into the
neighbouring column.

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

Two things about that menu are worth knowing, because both are invisible until
you look at the bytes:

- **It contains no newlines.** dgamelaunch places every entry with
  `ESC[8;3Hl) Login ESC[9;3Hr) Register new user`, so with the escape codes
  stripped the whole screen is one line. Matching `l)` at the start of a line
  never fires against a real server.
- **A rejected password says nothing.** nethack.alt.org simply redraws the
  "Not logged in." menu. Watching for the words "login failed" would wait
  forever, so the menu coming back *after* the password is submitted is what
  counts as a rejection, and the account name appearing is the confirmation.

The status bar distinguishes the two logins: the SSH connection is the shared
account, and "Logged in to the game server as …" is yours.

A first run with no config file at all starts with nethack.alt.org and
hardfought.org already listed, each pointed at a tile sheet matching the
NetHack line that server runs. Deleting every profile is a choice, not a first
run, so they are not handed back.

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
