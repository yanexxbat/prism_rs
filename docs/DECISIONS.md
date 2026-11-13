# Decisions

Where the port had a choice, it took the simpler variant. Each entry says what the
original does and what the port does instead.

## Direct launch instead of `NewLaunch.jar`

The original starts `org.prismlauncher.EntryPoint` from its own Java library and sends
the main class, game arguments and session over stdin. The port starts the game's main
class directly and passes the game arguments on the command line, the way the Mojang
launcher does. The port appends what the Java wrapper would have appended: `--width`,
`--height`, and the server or world to join.

Consequences:

- The access token is on the Java command line, so other processes of the same user
  can read it while the game runs. It is censored in the log.
- Versions that need the legacy applet wrapper (traits `legacyLaunch`, `alphaLaunch`,
  roughly Minecraft 1.5.2 and older) are refused with an error.
- The system properties `org.prismlauncher.launch.mainclass` and
  `org.prismlauncher.launch.gameargs` are not set.
- The "online fixes" for legacy versions are not available.

## Java check without `JavaCheck.jar`

The original runs a small Java program that prints `os.arch`, `java.version` and
`java.vendor`. The port runs `java -XshowSettings:properties -version` and reads the
same three properties from its output. This works on Java 7 and newer.

## No automatic Java download

The original can download the Mojang Java runtime a version asks for. The port only
finds installed runtimes (registry, `JAVA_HOME`, `PATH`, `<data>/java`, the Mojang
launcher's runtime folder) and picks the first one whose major version the game
accepts. If none fits, the launch fails with the message from `VerifyJavaInstall`.

If the global `JavaPath` is empty, the port uses the newest runtime it found. The
original asks in its setup wizard instead.

## Own data folder by default

The default data folder is `%APPDATA%\prism_rs` (`~/.local/share/prism_rs` elsewhere),
not Prism's folder, so the port never writes to an existing Prism installation unless
asked to. `--dir <path>` points it at any folder with Prism's layout.

## Client ID and API key come from the user

No Microsoft client ID and no CurseForge API key are built in. They are read from the
environment (`PRISM_RS_MSA_CLIENT_ID`, `PRISM_RS_CURSEFORGE_API_KEY`) or from the
settings keys the original already has (`MSAClientIDOverride`, `FlameKeyOverride`).
Without a client ID, Microsoft login is disabled; without an API key, CurseForge is
disabled.

## Offline accounts keep the original's ownership rule

As in the original, an offline account can only be added once a Microsoft account
that owns Minecraft is present. The test harness in `crates/minecraft/tests/launch.rs`
builds an offline session directly and does not go through the account list.

## Metadata cache without ETags

The original keeps ETags in `metacache`. The port uses a local metadata file when its
SHA-256 matches the one the parent list announces. Files with no announced hash are
fetched again, and the local copy is the fallback when the request fails.

## INI files

- One parser reads both the current format (`[General]`, Qt escapes) and the old
  MultiMC format (no section header).
- An unquoted value with commas stays one string. Qt would turn it into a list.
- Values are written with Qt's escaping rules; keys are written sorted, as Qt does.

## Mods

- A downloaded mod is written to the instance's `mods` folder. The packwiz metadata
  the original writes to `mods/.index` is not written, which matches the original
  with `ModMetadataDisabled` turned on.
- Dependencies of a mod are not resolved automatically.
- Only mods are browsed, not resource packs, shaders or modpacks.

## Not ported, on purpose

Jar mods, the legacy FML library task, pre-launch/wrapper/post-exit commands, instance
import and export, instance groups editing, skins, world and screenshot management,
translations, themes, the updater.

## Smaller deviations

- `MaxMemAlloc` defaults to 4096 MiB. The original uses RAM / 1.5 on machines with
  less than 6 GB.
- The log window always opens on launch. The original only shows it when `ShowConsole`
  is set or the game crashes.
- An account that does not own Minecraft cannot launch. The original offers the demo.
- The Microsoft token response is not stored as a whole (`msa.extra` stays empty), and
  the issue and expiry times of the two Xbox tokens are not parsed. Nothing reads them.
- Deleting an instance or a mod only moves it to the recycle bin. If that fails, nothing
  is deleted.
- `lastLaunchTime` and play time are not recorded.
- Instance groups from `instgroups.json` are shown but cannot be edited.
- Keys in a saved INI file are sorted. Qt keeps the order in which keys were added.
- The SRV lookup for "join server on launch" is not done; the address is passed as typed.

## How the GUI is tested

The UI code runs in `App::ui`, which needs no window. The tests in
`crates/app/src/gui_tests.rs` drive it with `egui_kittest`: they find buttons and fields
through egui's accessibility tree and click them. This is why the `accesskit` feature
of eframe is enabled. A real window only repaints while it is visible, so scripted clicks
on the window itself were not reliable.
