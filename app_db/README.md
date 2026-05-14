# wemu App Compatibility DB

This directory stores human-readable Markdown entries with machine-readable YAML front matter.

The front matter is the source of truth for launch tooling, website compatibility
lists, and future auto-configuration. Body text is for maintainers and website
copy.

## Files

- `schema.md`: field definitions and conventions.
- `_template.md`: starting point for a new app entry.
- `rich4.md`: Rich4 compatibility entry.

## Compile

Generate the browser-facing JSON with:

```bash
python3 tools/compile_app_db.py
```

The compiler validates entries and writes `web/app_db.json`. Use
`python3 tools/compile_app_db.py --check` in CI to verify the generated file is
up to date.

## Naming

Use lowercase, stable IDs for filenames:

```text
app_db/rich4.md
app_db/freecell.md
app_db/notepad.md
```

The `id` field must match the filename without `.md`.

## Status Values

- `active-target`: actively used to guide emulator work.
- `ui-target`: used for USER/GDI/dialog/text coverage.
- `regression`: automated or semi-automated regression fixture.
- `boots`: reaches first usable screen.
- `playable`: basic gameplay or workflow is usable.
- `broken`: known not to run.
- `unknown`: entry exists, but current status is not verified.

Prefer a conservative status. Do not mark an app `playable` without a recent visual or replay check.
