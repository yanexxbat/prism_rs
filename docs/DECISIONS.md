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

## Client ID and API key

The Microsoft client ID of the original is not used. The port has its own built-in ID
(`DEFAULT_MSA_CLIENT_ID`), added on the owner's request; it can be replaced. No CurseForge
API key is built in. Both are read from the
environment (`PRISM_RS_MSA_CLIENT_ID`, `PRISM_RS_CURSEFORGE_API_KEY`) or from the
settings keys the original already has (`MSAClientIDOverride`, `FlameKeyOverride`).
Without an API key, CurseForge is disabled.

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

## The glass look

The window is a pane of dark frosted glass: the desktop shows through it, blurred. It does
not follow the original's Qt themes. How it is built:

- The window is undecorated and transparent. Windows' acrylic backdrop blurs what is
  behind it; egui adds a smoky tint, a soft sheen from above, film grain and a light
  1 px edge. On Windows the window is clipped (`SetWindowRgn`) to its rounded shape, so
  the corners are really transparent. Without acrylic (other systems, or if the call
  fails) the tint is denser, since the desktop would show through sharp.
- The desktop behind the glass may be white, and text contrast comes first:
  `glass::tests::text_stays_readable_over_a_white_desktop` checks every text colour on a
  white desktop (4.8:1 for the dimmest). That decides how dark the glass is; over a dark
  desktop it looks nearly black.
- The glass repaints only when the UI changes; nothing animates while idle.

Changes of layout, not of function: the toolbar became a header with a title, the
"Add Instance" and "Reload" buttons and a navigation (Instances, Accounts, Mods, Settings).
"Mods" moved from the instance actions into the navigation and needs a selected instance.
Launch, Folder, Log and Edit are round icons at the bottom right of the instance details;
their accessible names are unchanged, as are the instance tiles' (the old list text).

## `unsafe` code

- `crates/app/src/acrylic.rs` calls `SetWindowCompositionAttribute` from user32 to blur
  what is behind the window. It is undocumented, so it needs raw FFI. The documented
  ways were tried first and are opaque on Windows 11: the system backdrop goes solid when
  the window is inactive, and the plain blur accent renders as a dark layer. If the call
  is missing, the window stays see-through without blur. The same file clips the window
  with GDI regions.
