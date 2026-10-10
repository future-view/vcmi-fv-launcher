# VCMI-FV-Launcher

A standalone desktop launcher that **does not modify VCMI source code**. It starts from a minimal baseline, saves VCMI-native `modSettings.json` presets, and launches the game / map editor in one click.

## Features

- **Mods view**: search, filter, and pick mods in the gallery; selected mods grouped by type with auto dependency checking and conflict highlighting.
- **Preset management**: searchable preset switcher, save-on-change, and a menu for new (with inheritance), rename, import/export, delete, and Markdown activation report.
- **Wiki**: offline browsing of 9 categories (towns / creatures / heroes / buildings / artifacts / spells / skills / terrains / mods) with right-click hover popups.
- **Realms**: multiple isolated environments (mods + saves + settings + maps), exportable as `.vcmirealm` and installable from remote repositories.
- **Settings panel**: official 12-section schema plus launcher-owned settings (AI super-resolution / HD edition assets / mirrors / updates).
- **Multi-language**: launcher UI in Chinese / English / Russian / Polish / German; mod and wiki text resolved by VCMI language identifier; built-in help in 5 languages.
- **Bottom bar**: language dropdown, game data drawer (structured dirs + health check), help drawer, map editor entry, and launch button.
- **Directory detection**: reads `dirs.json` with default fallback; reads VCMI config dir to write presets.
- **Fault tolerance**: password-protected `Content.zip` falls back to color blocks on error without crashing.

## Tech Stack

- Frontend: React 18 + TypeScript + Tailwind CSS 3.4 (Vite 5, glassmorphism dark theme)
- Desktop shell: Tauri 2 (Rust), spawning `VCMI_client.exe` / `VCMI_mapeditor.exe`
- Zip: yauzl on TS side; zip crate on Rust side

## Development

```bash
# Install dependencies
npm install

# Frontend mock (browser)
npm run dev

# Verify real modSettings read/write & backup (uses temp copy, no real files changed)
npm run verify:settings

# Run Tauri desktop app
npx tauri dev

# Build web bundle
npm run build

# Build Windows exe
npx tauri build
```

## Documentation

Full index: [docs/README.md](docs/README.md) (English) / [docs/README.zh-CN.md](docs/README.zh-CN.md) (Chinese). Documents are written in Chinese and marked `.zh-CN.md`.

- `docs/architecture/` — overview, frontend, backend, IPC contract, persistence layout
- `docs/features/` — one document per feature (mods / wiki / realms / settings / HD / updates …)
- `docs/guides/` — build & run, release, portable deployment, repository index, icon replacement
- `docs/reference/` — VCMI directory mechanics, error codes, preset format
- `docs/decisions/` — architecture decision records (ADR)
