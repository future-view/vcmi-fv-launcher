# VCMI-FV Launcher Help

> This documentation ships with the launcher and updates with it; switching the interface language shows the matching translation.

## Getting started

### First launch

- The setup wizard opens on first launch: choose the **game directory** (must contain `VCMI_client.exe`) and the **user data directory** (usually `Documents\My Games\vcmi`), then save.
- Directories are detected from `config/dirs.json` in the game directory first, falling back to VCMI defaults.

### Four steps to play

1. Top bar → **Realms**: create or enter a realm (optional; without one the global configuration is used).
2. In **Mods**, pick towns/expansions on the left and review them on the right (dependencies are ticked automatically).
3. Click **Save** in the preset bar to write `modSettings.json`.
4. Click **Launch Game** in the bottom bar.

### Bottom bar at a glance

- Left: Import files / Game data / Help / Native launcher.
- Right: Map editor / Launch game.

## Mods and presets

### What is a preset

- A preset is a set of mod switches (the `mods` array plus sub-mod `settings[parent][child]`) stored in VCMI's `modSettings.json`.
- The preset bar supports: switch, save (activates once changes are detected), new (optionally inheriting from another preset), rename, import/export of the official single-preset format, delete, and a Markdown enable report.

### Dependencies and conflicts

- Ticking a mod auto-ticks its `depends` (hard dependencies); `conflicts` are highlighted red in the right pane.
- Missing mods, unmet dependencies or conflicts block launching and are listed in a dialog.

### Sub-mods

- Expand a parent mod and tick sub-items; they are written to `settings[parent][child]`. Sub-IDs keep spaces and dots (e.g. `minotaur warlock.new mino warlock`).

### Playable baseline

- Saving merges the minimal baseline (enhanced-edition defaults `vcmi` / `h3e-main-menu` / `core`) so the game stays launchable.

## Realms

### How isolation works

- A realm is an independent VCMI environment: the launcher rewrites `config/dirs.json` in the game directory (only the five keys VCMI understands: `userDataPath` / `userConfigPath` / `userCachePath` / `userLogsPath` / `userSavePath`) so user folders point into the realm.
- Deactivating restores the **original backup** (not "official default paths").
- Realm data lives under `%LOCALAPPDATA%\VCMI-FV-Launcher\realms\<id>\` by default, never inside the game directory.

### Three ways to reference mods

- **Default**: use mods from the user and game directories directly, without copying.
- **Bundled**: copy mods into the realm so they travel with it.
- **Borrowed**: link mods via a directory junction; un-referencing removes only the link.

### Maps valve

- Enabling "Block game-provided maps" renames the game `Maps` folder to `Maps.vcmi-fv-native` and creates a junction (fully transparent by default); disabling removes the junction. It can be restored at any time.

### Entering and leaving

- Entering performs activation (rewrites dirs.json, seeds the realm preset, materialises borrowed mods); leaving deactivates and restores.
- The realm UI has no bottom bar: launch and editor entries live inside the realm home screen.

## Settings panel

### How to open

- The gear button at the top right toggles settings; click it again (or click any tab) to leave. Settings is not a tab.

### Display level

- Three levels: **Common / Advanced / All** control which advanced and all-level items are visible (default: Common).
- The search box matches items across all levels — a handy shortcut to hidden items.

### Scope state

- The "Global shared / Realm-only" label at the top right is a **read-only state**: hovering or clicking expands a panel explaining which config directory these settings are written to, with a copy-path button.
- The launcher settings page is always "Global shared"; the realm settings page follows that realm's config policy.

### More menu

- **Runtime info**: recent map/save, game data language, enabled mods and VCMI version read from the current config directory.
- **Check for updates**: shown on the launcher settings page only.
- **Open config folder**: opens the current config directory in Explorer.
- **Reset to defaults** (red): restores every launcher-managed VCMI setting in that directory to built-in defaults — use with care.

### Write semantics

- Only values that differ from defaults are written to `settings.json`; changing an item back to its default deletes the key and hands it back to VCMI's built-in defaults.

## Updates

### How you learn about a new version

- Gear badge: visible on any page.
- "More" badge plus a version tag inside the menu: visible in settings.
- Auto dialog: the same version prompts **at most once per 7 days**; a **new** version prompts immediately.
- "Skip this version" ignores that version permanently (badge disappears); "Check for updates" still shows the result.

### Forced updates

- If the manifest declares a minimum supported version and yours is older, the launcher blocks with a full-screen overlay until you update or exit.

### Download and install

- The dialog downloads and verifies sha256; config-only packages apply in place (no restart), while packages containing the exe generate a replace script and restart the launcher.

## HD assets

### Source

- When the official HD edition asset mod (`hd-edition`) is detected, `@2x`/`@3x` variants prefer true HD generation, and town screens / creature backgrounds use HD-sized base images; other categories keep the original base.
- Settings → "FV launcher" (switch to the "All" level) changes the source (auto / off) and scale (auto / x2).

### AI upscaling

- With "AI HD enhance" on, illustration-like assets (creatures / heroes / buildings) use AI upscaling; pixel art still uses xBRZ.
- AI requires the "AI runtime" component and an **explicitly selected model** (no model = AI is not used).

### When changes take effect

- Changing a switch or scale requires **rebuilding the asset cache** to take effect.

## Network

### GitHub proxy

- If GitHub is unreachable, set an HTTP(S) proxy under Settings → "FV launcher" → "GitHub network proxy" (e.g. `http://127.0.0.1:7890`); empty means direct.
- It applies to GitHub URLs only: direct first, proxy on connection failure. SOCKS5 is not supported (use your client's mixed port).

### Mirror rules

- The "GitHub mirror rules" card maintains a mirror list: after a direct failure, mirrors are tried in order, then the proxy.

## Directories and data

### dirs.json mechanics

- VCMI understands only five keys (`userDataPath` / `userConfigPath` / `userCachePath` / `userLogsPath` / `userSavePath`); other fields are silently ignored.
- Paths expand environment variables; relative paths resolve against the process working directory, so absolute paths are required.
- The game directory `Mods` and the user directory `Mods` are mounted **at the same time**; the same applies to maps.

### Game data drawer

- The bottom bar "Game data" shows: engine version, core mod version (not the engine version), OS, data directories (open in Explorer) and a data integrity health check (LOD entries / core mod / asset containers).

### Importing files

- Bottom bar "Import files", or drop files onto the window: `zip` / `h3m` / `h3c` / `vmap` etc. Zip archives are recognised as mod packages and installed into the user mods directory.

## FAQ

### "Missing mod" warning

- A preset references a mod that is not on disk (moved or deleted). Add it on the Mods page or install it from the repository; for missing language mods the notice offers an "Install from repository" button.

### How is the VCMI version detected?

- The exe carries no PE version: the launcher probes `VCMI_client.exe -v`, then the PE version, then a scan. The core mod version (e.g. `1.5`) shown in Game data is the built-in mod asset version, not the engine version.

### Portable deployment

- Put the launcher exe anywhere (including next to the game). Point it at your directories in the wizard; `launcher.json` can pre-seed language, directories and switches for distribution.

### Why do update prompts sometimes not appear?

- Optional updates are throttled: each version prompts once when first discovered, then stays quiet for 7 days (badge and menu tag remain). Use "More → Check for updates" at any time.
