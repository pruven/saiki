# Saiki

![Saiki showing a typed word and two shortcuts](preview.png)

Saiki shows your keystrokes in a click-through overlay for demos and screen recordings on Wayland and Windows. Rounded charcoal cards stack newest first along either screen edge and fade after the selected delay. Saiki renders them with [GPUI](https://gpui.rs/). The card style and motion take cues from [Hokum’s notifications](https://github.com/anomalyco/hokum).

The build pins a [published snapshot of Zed’s GPUI](https://github.com/gpui-archipelago/gpui-unofficial) for its Wayland layer-shell and click-through APIs.

## Install on Linux

Download `saiki-linux-x86_64.tar.gz` or `saiki-linux-aarch64.tar.gz` for your CPU from [Downloads](https://github.com/pruven/saiki/releases/latest). Extract the archive, open a terminal in the extracted folder, and run:

```sh
sh install.sh
```

This installs Saiki in `~/.local/bin` and adds its icon to your application launcher. Open **Saiki** from the launcher; the extracted download can then be deleted. No Rust toolchain or administrator access is needed to install the app. Quit Saiki before reinstalling or updating. To uninstall, quit Saiki and run `~/.local/bin/saiki-uninstall`; your settings are kept.

The Linux builds require glibc 2.39 or newer and the [Linux runtime dependencies](#linux-requirements). For older distributions, build from source.

### Arch packages

Download the `x86_64` package for Arch Linux from [GitHub Releases](https://github.com/pruven/saiki/releases/latest). From the download directory, install it with `sudo pacman -U ./saiki-*-x86_64.pkg.tar.zst`. ARM64 users can use the Linux archive described above.

Open Saiki from your launcher or run `saiki`.

## Install on Windows

Download `saiki-windows-x86_64-setup.exe` for Intel/AMD PCs or `saiki-windows-arm64-setup.exe` for ARM PCs from [Downloads](https://github.com/pruven/saiki/releases/latest). Open the setup file and follow the installation wizard. Then open **Saiki** from the Start menu. Setup installs for your account without administrator access, offers an optional desktop shortcut, and adds Saiki to **Settings → Apps** for uninstalling. To update, quit Saiki from the tray and run the newer setup file; your settings are kept. ZIP downloads remain available for portable use.

The x64 build requires Windows 10 version 1607 or newer; the ARM64 build requires Windows 11. Both require a Direct3D 11 capable GPU. Capture runs without administrator access. The overlay appears on the primary display and stays above ordinary application windows; exclusive fullscreen applications may cover it.

## Updates

Saiki checks GitHub for a stable release when you first open the controls. Use **Check for updates** to check again. When a newer release is available, **Release notes** opens the release notes. **Download update** opens the installer or installation package for your platform in your browser, preferring Windows setup over the portable ZIP. Follow the installation instructions above to apply a downloaded update.

## Run from source

On Windows, install [Rust](https://www.rust-lang.org/tools/install) with the MSVC toolchain and Visual Studio Build Tools with **Desktop development with C++** and the Windows SDK. In PowerShell:

```powershell
.\bin\build.ps1
.\build\saiki.exe
```

For a native ARM64 build, select the ARM64 MSVC Rust toolchain and install the Visual Studio **MSVC ARM64/ARM64EC build tools** component along with the Windows SDK. On an ARM PC, run:

```powershell
rustup default stable-aarch64-pc-windows-msvc
.\bin\build.ps1 -Target aarch64-pc-windows-msvc
.\bin\test.ps1 -Target aarch64-pc-windows-msvc
```

To cross-compile from an x64 Windows PC, install the Visual Studio **MSVC ARM64/ARM64EC build tools** component and the Windows SDK, then run `rustup target add aarch64-pc-windows-msvc` and `.\bin\build.ps1 -Target aarch64-pc-windows-msvc`. Run the resulting executable and tests on an ARM PC. Both builds write `build\saiki.exe`. The build script also locates the SDK's x64 shader compiler for GPUI when it is not on PATH.

Use `.\bin\test.ps1` to run tests and `.\build\saiki.exe --demo` to preview the overlay. Settings are saved in `%APPDATA%\saiki`. The tray menu provides Open, Start, Pause, and Quit; launching Saiki again opens the existing session, and `.\build\saiki.exe --quit` exits it.

On Linux:

```sh
bin/build
build/saiki
```

Click **Start capture**, then close the controls or switch apps. Saiki stays in the tray and continues showing keys. Use the tray menu to open Saiki, start or pause capture, or quit. Launching Saiki again opens the current session, and `saiki --quit` exits it from the command line.

Enable the shortcuts-only option to hide ordinary typing. Pausing clears the cards. To include the overlay in a recording, capture an entire monitor.

Letters appear together as you type. Space finishes a word, Backspace edits the current word, and shortcuts get their own cards.

Use **Overlay position** to pick a corner or the center of the top or bottom edge. Use **Fade delay** to choose how long cards stay visible after the last key, from 0.5 to 10 seconds. The default is 2.6 seconds. Saiki remembers both settings between launches.

Use **Super key display** to choose **Super** or **Windows icon** on either platform. The setting applies to standalone presses and shortcuts and is remembered between launches. Linux defaults to **Super** and Windows defaults to **Windows icon**.

Click **Preview** or run `build/saiki --demo` to show a sample stack.

| In the Saiki window | Action |
| --- | --- |
| Space | Start or pause capture |
| P | Open the position picker. Use arrows and Enter to select |
| [ / ] | Shorten or lengthen the fade delay |
| Q | Quit |
| ? | Show shortcuts |

Capture starts paused. On Linux it uses read-only access to the session’s keyboard devices; on Windows it uses a passive keyboard hook. Cards stay in memory. Pause capture before typing sensitive text. Saiki pauses on lock events and desktop inactivity reported by logind on Linux, and when the Windows session locks, disconnects, or switches to a secure desktop. Start capture again after returning to the desktop.

## Linux requirements

- A Wayland compositor with `zwlr_layer_shell_v1`, such as Hyprland, Sway, or niri.
- A D-Bus session bus and a StatusNotifier-compatible tray for the tray icon.
- Read access to the session keyboard’s `/dev/input/event*` device. Saiki reports missing access in its window.
- A Vulkan driver, libinput, libudev, Wayland client libraries, xkbcommon, libxcb, Fontconfig, FreeType, system fonts, OpenSSL 3, and CA certificates. On Arch, see [PKGBUILD](pkgbuild/PKGBUILD) for package names. On Ubuntu 24.04, install the runtime libraries with:

  ```sh
  sudo apt install libvulkan1 libinput10 libudev1 libwayland-client0 libwayland-cursor0 \
    libxkbcommon0 libxkbcommon-x11-0 libxcb1 libfontconfig1 libfreetype6 \
    libssl3t64 ca-certificates xkb-data fonts-liberation
  ```

The Linux archives include `runtime-packages-ubuntu-24.04.txt`, the runtime package list tested by CI. From the extracted directory on Ubuntu 24.04, you can also install it with `xargs sudo apt install -y < runtime-packages-ubuntu-24.04.txt`. Arch packages declare the OpenSSL library SONAMEs linked by the release binary, so pacman requires compatible libraries.

Saiki reads Hyprland’s main keyboard layout and active group when you start capture. On other compositors it uses xkbcommon’s defaults. Set `XKB_DEFAULT_LAYOUT`, `XKB_DEFAULT_VARIANT`, or `XKB_DEFAULT_OPTIONS` when launching Saiki to choose a layout explicitly. It shows physical keypresses and modifier chords. Text committed by IMEs and compose sequences is outside the overlay’s scope on either platform. Windows uses the foreground application’s keyboard layout.

## Package and test

Run `bin/install` to build and install the Arch package and launcher entry. Run `makepkg -f` inside `pkgbuild/` to build the package archive.

Run `bin/test` for keyboard layout, modifier, and stack ordering tests. Use `SAIKI_TEST_DEVICES=1 bin/test` from an active desktop session to test keyboard-device access.

To build a Windows installer locally, install Inno Setup 6, run `bin/build.ps1`, then `bin/package.ps1`. For an ARM64 binary, pass `-Arch arm64` to the packaging script. The setup file is written to `release/`.

To publish a release, match the versions in `Cargo.toml` and `pkgbuild/PKGBUILD`, update `Cargo.lock`, and push the matching `vX.Y.Z` tag. GitHub Actions builds and tests Linux and Windows on native x64 and ARM64 runners. It publishes Linux archives with install/uninstall scripts and launcher assets, x86_64 Arch packages, Windows setup installers, and portable Windows ZIPs with SHA-256 checksums. CI installs the finished Linux packages in fresh containers with runtime dependencies and runs the installed executable with `--version`, checks for unresolved shared libraries, and checks the installed Windows executable with `--version`. These launch checks do not exercise the graphical overlay. Running the workflow manually builds downloadable artifacts.
