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

The GUI is a dark frosted-glass panel with chrome spheres and a torus behind it. It does
not follow the original's Qt themes. How it is built:

- The window is undecorated and transparent, and a little larger than the panel. The
  chrome that sticks out of the panel is drawn sharp in that margin. On Windows the
  window is clipped (`SetWindowRgn`) to the rounded panel and those spheres, so the
  corners are really transparent and clicks next to the panel reach what is behind.
- The chrome is pre-rendered (`crates/app/assets/chrome/render_chrome.py`, Blender and
  Cycles, lit by Blender's bundled CC0 studio HDRI) and drawn as sprites.
- The blur runs on the GPU through a glow paint callback: backdrop and sprites at a
  quarter of the size, four separable Gaussian passes (sigma 20 px). Without a glow
  context (tests, other renderers) the CPU computes the same picture. Either runs only
  when the panel size or the pointer parallax changes, so an idle window does not repaint.
- Text contrast decides the glass: the brief's tint, rgba(40, 40, 42, 0.78), left the
  grey captions at 3.2:1 over the bright chrome. The tint is 0.86, and
  `glass::tests::text_stays_readable_over_the_brightest_glass` checks every text colour
  against the brightest spot of the blurred panel (4.7:1 for the dimmest).
- The Windows acrylic blur is optional (Settings, Appearance, `GlassAcrylic` in
  prismlauncher.cfg). It blurs the desktop instead of the chrome, so the chrome is not
  drawn then, and the glass is darker still because the desktop may be white.

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
- `crates/app/src/blur.rs` makes OpenGL calls through glow for the blur.
