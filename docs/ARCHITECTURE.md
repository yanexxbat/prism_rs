# Architecture

This is a Rust port of [Prism Launcher](https://github.com/PrismLauncher/PrismLauncher)
(C++/Qt, about 117,000 lines in `launcher/`). The reference checkout is commit
`1a6438dcf4066e8ef66ca3577155cb760ff148db` (2026-11-12).

## Modules of the original

| Original (`launcher/…`) | What it does | Ported to | Status |
|---|---|---|---|
| `settings/` (`INIFile`, `SettingsObject`, `OverrideSetting`) | `prismlauncher.cfg` and `instance.cfg` in Qt INI format, per-instance overrides gated by `Override…` keys | `crates/core` (`ini`, `settings`) | ported |
| `BaseInstance`, `InstanceList`, `FileSystem` | Instance folders, ids, `instgroups.json`, folder naming | `crates/core` (`instance`) | ported (groups are read, not edited) |
| `Version` | Version string ordering used when merging libraries | `crates/core` (`version`) | ported |
| `net/` (`NetJob`, `Request`, `ChecksumValidator`, `HttpMetaCache`) | Concurrent downloads, checksum validation, ETag cache | `crates/net` | ported without the ETag cache |
| `meta/` (`Index`, `VersionList`, `Version`, `JsonFormat`) | Metadata server client (`meta.prismlauncher.org/v1`), sha256-checked local cache in `meta/` | `crates/minecraft` (`meta`) | ported |
| `minecraft/` (`VersionFile`, `OneSixVersionFormat`, `MojangVersionFormat`, `Library`, `Rule`, `GradleSpecifier`) | Component file format, library rules, natives, download URLs | `crates/minecraft` (`version_file`) | ported |
| `minecraft/` (`PackProfile`, `Component`, `ComponentUpdateTask`, `LaunchProfile`) | `mmc-pack.json`, `patches/<uid>.json`, dependency resolution, merged launch profile | `crates/minecraft` (`pack_profile`) | ported |
| `minecraft/update/` (`FoldersTask`, `LibrariesTask`, `AssetUpdateTask`), `AssetsUtils` | Libraries, asset index, asset objects, virtual assets | `crates/minecraft` (`launch`, `assets`) | ported; `LegacyFMLLibrariesTask` not ported |
| `minecraft/launch/`, `launch/`, `MinecraftInstance` | Launch steps: Java check, natives extraction, JVM and game arguments, process and log | `crates/minecraft` (`launch`) | ported; see below |
| `java/` (`JavaUtils`, `JavaChecker`, `JavaVersion`, `JavaInstallList`) | Find Java installs, read version and architecture, pick a compatible one | `crates/minecraft` (`java`) | ported; automatic Java download not ported |
| `minecraft/auth/` (`AccountData`, `AccountList`, `AuthFlow`, `steps/*`) | `accounts.json`, Microsoft login (device code and browser), Xbox Live, XSTS, Minecraft services, offline accounts | `crates/auth` | device-code flow, refresh and offline ported; browser flow and skins not ported |
| `modplatform/modrinth`, `modplatform/flame` | Modrinth and CurseForge search, version lists, downloads | `crates/minecraft` (`modplatform`) | mods only: search, file list, install |
| `modplatform/` (`atlauncher`, `ftb`, `technic`, `packwiz`, `import_ftb`, `legacy_ftb`) | Modpack platforms, packwiz mod metadata | – | not ported |
| `ui/` (`MainWindow`, `InstanceWindow`, `pages/`, `dialogs/`, `themes/`, `setupwizard/`) | Qt Widgets GUI | `crates/app` (egui) | simplified: instance list, accounts, settings, launch, log, mod browser |
| `libraries/launcher` (Java `NewLaunch.jar`) | Java-side wrapper that receives the launch script on stdin and starts the game | – | replaced by a direct launch, see `DECISIONS.md` |
| `libraries/javacheck` (Java) | Prints Java system properties | – | replaced by `java -XshowSettings:properties -version` |
| `InstanceImportTask`, `InstanceCopyTask`, `MMCZip`, `minecraft/World*`, `screenshots/`, `news/`, `updater/`, `translations/`, `tools/`, `icons/`, `logs/` | Import/export, copies, worlds, screenshots, news, updater, translations, profilers, icon themes | – | not ported |

## Crates

```
crates/
  core       prism-core       INI format, settings with overrides, instances, version ordering
  net        prism-net        HTTP client, downloads with SHA-1/SHA-256 validation
  auth       prism-auth       accounts.json, Microsoft device-code login, refresh, offline accounts
  minecraft  prism-minecraft  metadata, components, libraries, assets, Java, launch, mod platforms
  app        prism-app        egui GUI and the command line (--dir, --launch, --profile, --offline)
```

Dependencies point one way: `app` → `minecraft` → `auth`, `net`, `core`.

## Data layout

The layout is the one Prism uses, so an existing data folder works with `--dir`:

```
<data>/prismlauncher.cfg        global settings (Qt INI)
<data>/accounts.json            accounts, format version 3
<data>/meta/<uid>/<version>.json   cached metadata
<data>/libraries/…              Maven-layout library store
<data>/assets/{indexes,objects,virtual}
<data>/java/                    scanned for Java runtimes
<data>/instances/<id>/instance.cfg
<data>/instances/<id>/mmc-pack.json
<data>/instances/<id>/patches/<uid>.json   local component overrides
<data>/instances/<id>/minecraft/ (or .minecraft/)   game folder
<data>/instances/<id>/natives/  extracted for each launch, removed afterwards
```

## Launch sequence

The order follows `MinecraftInstance::createLaunchTask`:

1. Create the game folder.
2. Load `mmc-pack.json`, load every component (local patch or metadata), resolve dependencies.
3. Pick and check Java; verify it against `compatibleJavaMajors`.
4. Online only: download libraries (SHA-1 checked), the asset index and asset objects.
5. Print the instance description to the log.
6. Extract native jars into `natives/`.
7. Rebuild virtual assets when the asset index asks for it.
8. Start Java, forward its output to the log with tokens censored.
9. Remove `natives/` when the game exits.
