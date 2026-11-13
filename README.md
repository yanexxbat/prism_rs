<div align="center">

# prism_rs

**A Minecraft launcher in Rust, ported from [Prism Launcher](https://github.com/PrismLauncher/PrismLauncher).**

[![License: GPL-3.0](https://img.shields.io/badge/license-GPL--3.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg)](https://www.rust-lang.org)
[![Platform: Windows](https://img.shields.io/badge/platform-Windows%20x64-lightgrey.svg)](#download)
[![Release](https://img.shields.io/github/v/release/yanexxbat/prism_rs)](https://github.com/yanexxbat/prism_rs/releases/latest)

</div>

prism_rs manages and starts Minecraft instances. It reads and writes Prism Launcher's
data formats (`instance.cfg`, `mmc-pack.json`, `accounts.json`, the metadata format of
`meta.prismlauncher.org`), so instances made by Prism Launcher work. The original is
C++/Qt; this port is Rust with an [egui](https://github.com/emilk/egui) interface and
ships as a single `.exe`.

> **Status: early release (v0.1.0).** The core flows work and were tested against the
> real game on Windows (see [Tests](#tests)). Several features of the original are not
> ported yet (see [What is missing](#what-is-missing)).

## Highlights

- **Instances:** create, edit and delete; per-instance settings override global ones
- **Versions and loaders:** vanilla, Fabric, Quilt, Forge and NeoForge, resolved from the
  Prism metadata server
- **Launch:** JVM arguments matched Prism Launcher's own log in a side-by-side comparison;
  live log window with tokens censored
- **Accounts:** Microsoft device-code login and offline accounts
- **Mods:** search, install and manage mods from Modrinth and CurseForge
- **Compatible:** works on Prism Launcher instance folders (`--dir`)
- **Command line:** launch an instance directly and join a server or world

## Download

Get `prism_rs-v0.1.0-windows-x64.zip` from the
[latest release](https://github.com/yanexxbat/prism_rs/releases/latest), unzip it and run
`prism_rs.exe`. The program is not code-signed, so Windows SmartScreen may warn on the
first start. You need a Java runtime that fits your Minecraft version (see below).

## Project layout

| Crate | Purpose |
|---|---|
| `crates/core` | settings, INI files, instances, versions |
| `crates/net` | downloads with checksum verification |
| `crates/auth` | accounts, Microsoft/Xbox/Minecraft login, sessions |
| `crates/minecraft` | metadata, components, libraries, assets, Java, launch, loaders, mod platforms |
| `crates/app` | the egui GUI and the command line |

More detail:

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): the modules of the original and where each one went
- [`docs/DECISIONS.md`](docs/DECISIONS.md): every place where the port does something simpler than the original

## Licence and origin

This is a derivative work of Prism Launcher under GPL-3.0. See [`LICENSE`](LICENSE) and
[`NOTICE.md`](NOTICE.md). It is not affiliated with the Prism Launcher project. "Minecraft"
is a trademark of Mojang Synergies AB.

## Build on Windows

1. Install the MSVC build tools: "Visual Studio Build Tools 2022" with the workload
   "Desktop development with C++".
2. Install Rust: `winget install Rustlang.Rustup`, then
   `rustup default stable-x86_64-pc-windows-msvc`.
3. Build:

   ```
   cargo build --release
   ```

   The program is `target\release\prism_rs.exe`. Built and tested with Rust 1.100.0.

To play, you also need a Java runtime that fits the Minecraft version (Java 25 for 26.x,
Java 8 for 1.12.2 and so on). The launcher finds installed runtimes and runtimes placed
in the `java` folder of its data folder. It does not download Java.

## Run

```
prism_rs                       start the launcher
prism_rs --dir <folder>        use another data folder
prism_rs --launch <instance>   start the launcher and launch this instance (folder name)
         --offline <name>      ... offline, with this player name
         --profile <name>      ... with this account
         --server <address>    ... and join this server
         --world <name>        ... and join this world
```

The data folder is `%APPDATA%\prism_rs`. It has the same layout as Prism Launcher's
folder. To use the instances of an existing Prism Launcher installation, either copy
the instance folders into `%APPDATA%\prism_rs\instances`, or start with
`--dir %APPDATA%\PrismLauncher`. With `--dir`, prism_rs writes to that folder like Prism
would: it stores a Java path in `instance.cfg` on launch, and it saves `accounts.json`
when you change accounts. Microsoft accounts stored by Prism Launcher cannot be reused,
because they are bound to Prism's client ID; log in again.

## Microsoft login

A client ID is built in, so the login works without setup. To use your own application instead, register one:

1. Open the [Azure portal](https://portal.azure.com), go to "Microsoft Entra ID" →
   "App registrations" → "New registration".
2. Supported account types: "Personal Microsoft accounts only". No redirect URI is needed.
3. In the new application, open "Authentication" and turn on "Allow public client flows".
4. Copy the "Application (client) ID".
5. New applications must be approved for the Minecraft API before the login works. Ask
   for approval with the form at <https://aka.ms/mce-reviewappid>.

Give your own ID to the launcher in one of two ways:

- environment variable `PRISM_RS_MSA_CLIENT_ID`
- Settings → "Microsoft client ID" (stored as `MSAClientIDOverride` in `prismlauncher.cfg`
  in the data folder)

The login uses the device-code flow: the launcher shows a code, and you enter it at the
address it names. As in the original, an offline account can only be added once a
Microsoft account that owns Minecraft is in the list.

## CurseForge

CurseForge needs an API key from <https://console.curseforge.com>. Set
`PRISM_RS_CURSEFORGE_API_KEY` or Settings → "CurseForge API key" (`FlameKeyOverride`).
Modrinth needs no key.

## What is ported

| Area | Ported |
|---|---|
| Settings | `prismlauncher.cfg` and `instance.cfg` in Qt INI format, per-instance overrides |
| Instances | list, create, edit, delete (recycle bin), groups shown |
| Metadata | index, version lists and version files from the metadata server, SHA-256 checked cache |
| Components | `mmc-pack.json`, `patches/<uid>.json`, dependency resolution, library merging by version |
| Libraries | rules per OS and architecture, old and new style natives, Maven and Mojang download URLs, SHA-1 checked downloads |
| Assets | asset index, objects, virtual and resource-mapped assets for old versions |
| Java | detection (registry, `JAVA_HOME`, `PATH`, launcher and Mojang runtime folders), version and architecture check, automatic choice by `compatibleJavaMajors` |
| Launch | JVM and game arguments in the original's order, environment, natives extraction, log with censored tokens, kill, join server or world |
| Accounts | `accounts.json` version 3, Microsoft device-code login, Xbox Live, XSTS, Minecraft services, ownership, profile, refresh, offline accounts and offline launch |
| Mod loaders | Fabric, Quilt, Forge, NeoForge as components, install and remove |
| Mods | Modrinth and CurseForge: search, file list, install; enable, disable, remove |
| GUI | instance list, accounts, settings, launch button, log window, mod browser, instance editor |
| Command line | `--dir`, `--launch`, `--profile`, `--offline`, `--server`, `--world` |

## What is missing

- Automatic Java download
- Minecraft 1.5.2 and older (they need the applet wrapper from `NewLaunch.jar`), jar mods,
  the legacy FML library task, online fixes for legacy versions
- Microsoft login through the browser (authorization-code flow), skins and capes, demo mode
- Modpacks (Modrinth, CurseForge, ATLauncher, FTB, Technic), instance import and export,
  instance copies, shortcuts
- Resource packs, shader packs, data packs, worlds, screenshots as managed lists
- Mod updates, mod dependencies, packwiz metadata in `mods/.index`
- Pre-launch, wrapper and post-exit commands, play time tracking
- Instance group editing, icons, themes, translations, news, the updater
- ETag caching of downloads; the proxy settings
- The Linux-only options (MangoHud, GameMode, discrete GPU, Zink)

## Tests

```
cargo test                                          unit tests, no network
cargo test -p prism-minecraft --test online -- --ignored
cargo test -p prism-minecraft --test launch -- --ignored --nocapture --test-threads=1
cargo test -p prism-app full_flow -- --ignored --nocapture
```

The ignored tests need network access; the last two start the real game and close it
again. They use the folder `_testdata` (or `PRISM_RS_TEST_DIR`) and scan
`PRISM_RS_TEST_JAVA_DIR` for Java. `PRISM_RS_TEST_INSTANCE=<instance folder>` makes
`existing_prism_instance_starts` launch a copy of that instance.

### Results, 2026-11-13, Windows 11, Rust 1.100.0

| Test | Result |
|---|---|
| Unit tests: core 10, net 2, auth 11, minecraft 31, app 3 | 57 passed |
| `online`: Fabric, Quilt, Forge and NeoForge resolve for 1.20.1 against the live metadata server | passed |
| `online`: Modrinth search, file list and install of Sodium, SHA-1 verified | passed |
| `launch`: vanilla 26.3 (the current release) downloads 78 libraries and 5149 assets and starts | passed, reached `Backend library: LWJGL version 3.4.3` |
| `launch`: Fabric Loader 0.20.1 on 26.3 starts | passed, logged `Loading Minecraft 26.3 with Fabric Loader 0.20.1` |
| `launch`: copy of an instance made by Prism Launcher (Forge 14.23.5.2859 on 1.12.2, LWJGL 2, Java 8) starts | passed, reached `OpenAL initialized` |
| GUI `full_flow`: create a Fabric instance, search and install Sodium, launch, read the log window, kill | passed |

The command line was compared with Prism Launcher's own log for the same instance
(Fabric 0.20.1 on 26.3, Java 25):

- JVM arguments: identical, in the same order
  (`-Duser.language=en -XX:StackShadowPages=32 -XX:HeapDumpPath=… -Xms512m -Xmx4096m -Djava.library.path=…/natives`)
- Classpath: the same 86 entries in the same order. The original has one more,
  `NewLaunch.jar`, which the port replaces by a direct launch.
- Main class: the original starts `org.prismlauncher.EntryPoint` and sends the rest over
  stdin; the port starts `net.fabricmc.loader.impl.launch.knot.KnotClient` with the game
  arguments on the command line.

Not tested:

- Microsoft login, refresh and an online-mode launch against the real servers (no client
  ID was available). The flow is tested against a local mock server.
- CurseForge against the real API (no API key). Requests and response parsing are unit tested.
- Starting the game with Quilt, NeoForge or modern Forge. Only their component lists were resolved.
- `--dir` on a live Prism Launcher folder, the "Launch Offline" button, deleting through
  the recycle bin, joining a server or world.
- Linux and macOS. The code has their paths, but only Windows was built and run.
- The launch tests use an offline session built in the test, not the account list.
