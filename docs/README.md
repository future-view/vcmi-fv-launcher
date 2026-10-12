# Documentation Index

> Documentation is written in Chinese (`.zh-CN.md`); this file is the English entry point.
> Content follows **the code as implemented** — when they disagree, the docs are wrong.

## Start here

| Document | What you get |
|---|---|
| [Overview](./architecture/overview.zh-CN.md) | Positioning, feature map, tech stack, source layout, glossary |
| [Persistence](./architecture/persistence.zh-CN.md) | Exactly what is written to disk |
| [IPC Contract](./architecture/ipc.zh-CN.md) | Command list and frozen contracts (read before touching the backend) |

---

## Architecture (cross-cutting)

| Document | Summary |
|---|---|
| [overview](./architecture/overview.zh-CN.md) | Positioning, feature map, tech stack, layout, doc conventions |
| [frontend](./architecture/frontend.zh-CN.md) | Four layers, placement rules, dependency direction, startup orchestration |
| [backend](./architecture/backend.zh-CN.md) | Rust layering, `infra` modules, concurrency, do-not-change list |
| [ipc](./architecture/ipc.zh-CN.md) | 136 commands, events, three-way parity, frozen contracts |
| [persistence](./architecture/persistence.zh-CN.md) | Data dir, `launcher.json` groups, realm / cache / log layout |

## Features

| Document | Summary |
|---|---|
| [shell-and-navigation](./features/shell-and-navigation.zh-CN.md) | Top bar, bottom bar, game data, help, windowing, render priority |
| [startup-responsiveness](./features/startup-responsiveness.zh-CN.md) | First-boot mods freeze: main-thread root causes and phased fixes (implemented 2026-10-08) |
| [setup-wizard](./features/setup-wizard.zh-CN.md) | First-run wizard: trigger, steps, directory detection |
| [mods](./features/mods.zh-CN.md) | Browsing and picking mods: dependency closure, conflicts, town backdrops |
| [presets](./features/presets.zh-CN.md) | Presets: native format conversion, import/export, activation report |
| [lang-mods](./features/lang-mods.zh-CN.md) | Language mod discovery by category (not by name) + pre-launch check dialog |
| [mods-repository](./features/mods-repository.zh-CN.md) | Fetching and updating mods from the official repository |
| [game-import](./features/game-import.zh-CN.md) | Importing maps / saves / mod zips / HD assets |
| [wiki](./features/wiki.zh-CN.md) | Encyclopedia UI: 9 categories, detail dispatch, right-click popup |
| [modcache](./features/modcache.zh-CN.md) | Cache pipeline: two phases, artifacts, cache keys, realm priority build |
| [encrypted-zip-handling](./features/encrypted-zip-handling.zh-CN.md) | Encrypted `content.zip` password unpacking and the two distribution feeding routes |
| [faction-showcase](./features/faction-showcase.zh-CN.md) | Faction showcase: deck scheduling, playback, audio sources |
| [settings](./features/settings.zh-CN.md) | Settings panel: 13 groups, scope badge, launcher-owned cards |
| [hd-assets](./features/hd-assets.zh-CN.md) | HD: `.pak` source / upscaling / AI super-resolution / external runtime |
| [network-egress](./features/network-egress.zh-CN.md) | Egress: direct → mirror → proxy fallback |
| [realms](./features/realms.zh-CN.md) | Realms: lifecycle, isolation, maps valve, self-check, import/export |
| [realms-editor](./features/realms-editor.zh-CN.md) | Realm editor: six tabs and save semantics |
| [realms-repository](./features/realms-repository.zh-CN.md) | Remote realm repository: index, state matrix, install verification |
| [launcher-update](./features/launcher-update.zh-CN.md) | Auto-update: manifest, forced vs optional, self-replace, config patch |
| [i18n](./features/i18n.zh-CN.md) | Three language systems: UI / VCMI content / help docs |

## Guides

| Document | Summary |
|---|---|
| [build-and-run](./guides/build-and-run.zh-CN.md) | Install, run, package, and verification scripts |
| [release](./guides/release.zh-CN.md) | ChangeLog syntax, local packaging, GitHub publish, `index.json` |
| [release-cross-platform](./guides/release-cross-platform.zh-CN.md) | Cross-platform release & update implementation plan (ADR 0008 companion): platform descriptors, per-target manifests, `updateMode` dispatch, usage demos |
| [portable-deployment](./guides/portable-deployment.zh-CN.md) | Portable data dir, bundled presets, AI flavor, known pitfalls |
| [distribution-build](./guides/distribution-build.zh-CN.md) | Custom distribution builds: built-in encrypted zip passwords (route B) and `launcher.json` config (route A) |
| [repository-index](./guides/repository-index.zh-CN.md) | Authoring and publishing a realm repository index |
| [replace-icon](./guides/replace-icon.zh-CN.md) | Which icon files to replace |

## Reference

| Document | Summary |
|---|---|
| [vcmi-dirs](./reference/vcmi-dirs.zh-CN.md) | Hard facts about VCMI directories (`dirs.json` keys, dual mounts, junctions) |
| [error-codes](./reference/error-codes.zh-CN.md) | `be_err(code,arg)` contract, common codes, guard levels |
| [mod-settings-format](./reference/mod-settings-format.zh-CN.md) | `modSettings.json` and preset format |

## Decisions (ADR)

| Document | Decision |
|---|---|
| [0001](./decisions/0001-realm-isolation-via-dirs-json.zh-CN.md) | Realm isolation rewrites the 5 real keys of `dirs.json` |
| [0002](./decisions/0002-maps-junction-valve.zh-CN.md) | Map blocking uses a junction valve, applied lazily |
| [0003](./decisions/0003-town-backdrop-single-source.zh-CN.md) | Town backdrops come only from `build_towns` output |
| [0004](./decisions/0004-modcache-index-no-prune.zh-CN.md) | Cache index merges only, never prunes |
| [0005](./decisions/0005-ai-runtime-external.zh-CN.md) | AI runtime is external, not statically linked |
| [0006](./decisions/0006-guard-coexist-not-block.zh-CN.md) | Guards are graded: coexistence over hard blocking |
| [0007](./decisions/0007-version-injection-at-pack.zh-CN.md) | `release:pack` injects the release version (persisted by default) |
| [0008](./decisions/0008-cross-platform-release-and-update.zh-CN.md) | Cross-platform release & update: platform descriptors + per-target manifests + `updateMode` dispatch (confirmed) |

## IDE Rules

The source of truth for coding rules lives in `docs/ide/rules/` (`NN-<topic>.mdc`), **independent of any IDE**. Copy them into your IDE's rules directory; copies are not maintained in the repository — always edit the source of truth.
