---
schema: wemu.appcompat.v1
id: rich4
name: Rich4
status: active-target
updated: 2026-05-11
executables:
  - id: main
    filename: rich4.exe
    role: primary
launch:
  executable: main
  cwd: exe-parent
locale:
  ansi_codepage: big5
required_assets:
  - id: media
    type: directory
    name: Media
    required: true
    purpose: game-media
    locator: named-directory
    mount:
      drive: D
      device: cdrom
verification:
  last_checked: null
  method: manual
  evidence: []
---

# Rich4

## Summary

Rich4 is an active compatibility target for legacy graphics, mouse input, timers, file mounting, and Traditional Chinese text paths.

## Required Configuration

- Launch executable: `rich4.exe`
- ANSI code page: `big5`
- Find a `Media` directory and mount it as guest drive `D:`
- Mark drive `D:` as `cdrom`

## Expected Layout

Native host layout commonly looks like:

```text
wemu/
  Game/
    rich4.exe
    ...
  Media/
    ...
```

The app DB entry asks launch tooling to find a directory named `Media`. The
tooling owns the search policy for host folders, mounted roots, and uploaded
archives.

## Native CLI Example

```bash
cargo run --release -- --mount C:=/path/to/wemu/Game --mount D:=/path/to/wemu/Media --cmdline 'C:\rich4.exe'
```

The `D:` mount should be exposed as a CD-ROM drive so game code that probes disc media sees the expected drive type.

## Known Behavior

- Uses Big5/Traditional Chinese text paths.
- Expects media data on a disc-like `D:` drive.
- Used as a primary visual/gameplay target for browser, SDL2, and headless replay work.
