# Contributor and agent guide

This file tells a new contributor how this project works. It applies to people
and to AI agents. `CLAUDE.md` is a symlink to this file.

Write in Simplified Technical English (ASD-STE100). Keep sentences short. Use
one word for one meaning.

## What the app does

The app plays NetHack on public servers and shows graphical tiles. It connects
over SSH. The servers send tile numbers in `vt_tiledata` escape codes. The app
reads those codes and draws 16x16 tiles over the map.

The game runs on the server. Scores, dumplogs, and ttyrecs stay correct.

The app can also run a local NetHack in a pseudo-terminal.

## Stack

- Tauri 2. The window uses the operating system webview.
- Frontend: React and TypeScript in `src/`.
- Backend: Rust in `src-tauri/src/`.
- Terminal: xterm.js with a canvas over it for the tiles.

## Layout

| Path | Contents |
|---|---|
| `src/lib/` | Pure frontend logic. Tests live beside each file. |
| `src/components/` | React components. `GameTerminal.tsx` owns the terminal and the tile canvas. |
| `src-tauri/src/session.rs` | The transport interface. SSH and local play share it. |
| `src-tauri/src/ssh.rs` | The SSH transport. |
| `src-tauri/src/local.rs` | Local play in a pseudo-terminal. Unix only. |
| `src-tauri/src/demux.rs` | Splits `vt_tiledata` escape codes out of the terminal stream. |
| `src-tauri/src/glyph.rs` | Decodes tile numbers and glyph flags. |
| `src-tauri/src/tileset.rs` | Tile sheet geometry and manifests. |
| `src-tauri/src/autologin.rs` | The dgamelaunch login state machine. |
| `src-tauri/src/profiles.rs` | Profile storage. Passwords go to the keychain. |
| `src-tauri/examples/` | Developer tools. `tiles2png` builds a tile sheet. `appicon` draws the icon. |
| `scripts/` | Release scripts. Tests live beside each file. |

## Commands

```sh
npm install
npm run app          # run the app with hot reload
npm run app:build    # build a packaged app
npm run dev          # frontend only, in a browser
```

```sh
npm run test:all     # both test suites
npm test             # frontend tests (vitest)
npm run test:backend # backend tests (cargo)
npm run check        # tsc --noEmit, then cargo check
npm run lint         # clippy, warnings are errors
```

`npm run dev` starts the frontend without Tauri. The Tauri commands do not
exist there. Use it for style work only.

## How to work

Write the test first. Run it. See it fail. Then write the code. A test that
passes on the first run proves nothing.

Find the cause before you write a fix. A fix for a symptom hides the defect.

When a test needs to know what a server sends, capture the bytes. Do not
invent them.

Comments must give the reason for the code. The code shows what it does.

Do not put a personal account name in the code. Use `username`.

## Rules you must know

**Tiles need two lines in the server `.nethackrc` file.** The file must contain
`OPTIONS=vt_tiledata` and `OPTIONS=windowtype:tty`. Only the tty window port
sends tile codes. The curses port sends none.

**Tile numbers are positional.** A sheet from the wrong NetHack version draws
the wrong picture for almost every glyph.

**Hardfought needs a regional host.** Use `us.hardfought.org`, `eu.`, or `au.`.
The bare domain goes through a proxy that cannot accept SSH.

**Most local NetHack builds send no tiles.** `TTY_TILES_ESCCODES` is a
compile-time option. Packaged builds usually omit it. Such a build plays in
ASCII.

**A pseudo-terminal must have the close-on-exec flag.** If it does not, another
`fork` and `exec` inherits the file descriptor. The terminal then stays open
and the read never ends.

**Developer tools must stay in `examples/`.** The macOS bundler copies every
binary in the package into the app. A universal build merges only the main
binary. A second binary breaks `--target universal-apple-darwin`.

**Clippy treats warnings as errors.** An import that only a Unix build uses
must go inside the `#[cfg(unix)]` module. A module-level import breaks the
Windows build.

## Releases

Five files record the version. One command changes all of them.

```sh
npm run release              # 0.1.0 -> 0.1.1
npm run release -- minor     # 0.1.0 -> 0.2.0
npm run release -- --dry-run # show the changes, write nothing
```

Push the tag to start the build.

```sh
git push origin main v0.1.1
```

The workflow builds Windows and Linux. It attaches them to a draft release.

A maintainer builds macOS on a Mac with `npm run release:macos`. The Developer
ID key stays in that keychain. The key never goes into a repository secret.

Publish the draft last. Publication updates the Homebrew tap.

### Two things must both carry a notarisation ticket

The bundler notarises the `.app`. It then builds the `.dmg` around the app. The
disk image has no ticket at that point.

Homebrew does not see the problem. It downloads with curl, which sets no
quarantine flag, and copies out an app that has a ticket.

A browser sets the quarantine flag on the `.dmg`. The system then checks the
disk image, not the app inside it. An image without a ticket fails.

`npm run release:macos` notarises both. It checks both with `spctl` before it
uploads anything. Version 0.1.1 shipped without an image ticket. Do not remove
these checks.
