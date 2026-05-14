# App DB Schema

Schema version: `wemu.appcompat.v1`

Each app entry is a Markdown file with YAML front matter. The front matter is
the machine-readable source of truth; the Markdown body is for maintainers,
website copy, and verification notes.

```yaml
---
schema: wemu.appcompat.v1
id: example
name: Example App
status: unknown
updated: 2026-05-11

executables:
  - id: main
    filename: EXAMPLE.EXE
    role: primary

launch:
  executable: main
  cwd: exe-parent

locale:
  ansi_codepage: default

required_assets: []
---
```

## Required Fields

- `schema`: currently `wemu.appcompat.v1`.
- `id`: stable lowercase ID, matching the filename.
- `name`: display name.
- `status`: conservative compatibility status.
- `updated`: last entry update date, `YYYY-MM-DD`.
- `executables`: one or more executable match rules.
- `launch`: how tools choose the executable and working directory.
- `locale`: text encoding and locale overrides.
- `required_assets`: extra data directories or discs needed for auto-launch.

## Executables

```yaml
executables:
  - id: main
    filename: rich4.exe
    role: primary
```

- `id`: stable identifier used by `launch.executable`.
- `filename`: executable basename, not a host path.
- `role`: `primary`, `launcher`, `setup`, `helper`, or `test`.

Executable filenames and required asset names are always matched
case-insensitively. Do not add per-entry case-sensitivity knobs unless the
schema version changes.

## Launch

```yaml
launch:
  executable: main
  cwd: exe-parent
```

- `executable`: `id` of the executable entry to run by default.
- `cwd`: `exe-parent` means the guest working directory should be the selected
  executable's parent directory. A concrete guest path such as `C:\DATA` may be
  used when an app requires it.

## Locale

```yaml
locale:
  ansi_codepage: big5
```

- `ansi_codepage`: ANSI code page override for `A` APIs and legacy file names.
  Use `default` when no override is required. Prefer lowercase labels such as
  `big5` over platform-specific numeric aliases in app DB entries.

## Required Assets

```yaml
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
```

- `id`: stable identifier for diagnostics and UI.
- `type`: currently `directory` or `file`.
- `name`: expected host/archive entry name.
- `required`: whether missing media should block auto-launch.
- `purpose`: short human-readable reason, such as `game-media` or `setup-disc`.
- `locator`: currently `named-directory` for host/ZIP directory discovery by
  asset name. Launch tooling owns the search policy.
- `mount.drive`: guest drive letter without colon.
- `mount.device`: guest drive type, currently `fixed`, `cdrom`, or `virtual`.

## Notes

The app DB should describe reusable compatibility behavior and launch
requirements. Avoid app-specific emulator branches unless the entry is
documenting a known bug or missing API.
