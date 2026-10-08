# vdisplay

**English** | [简体中文](README.zh-CN.md)

Apollo-style virtual displays for [Sunshine](https://github.com/LizardByte/Sunshine).

`vdisplay` is a small Windows command-line tool meant to be used as a Sunshine **prep command**. When a stream starts it creates a virtual display that matches the client's resolution, refresh rate and HDR state, and removes it again when the stream ends — just like [Apollo](https://github.com/ClassicOldSong/Apollo) does, but with stock Sunshine.

It uses the same driver as Apollo: [SudoVDA](https://github.com/SudoMaker/SudoVDA) (SudoMaker Virtual Display Adapter). The driver is bundled inside `vdisplay.exe`.

## Features

- Virtual display created on demand with the client's **resolution, FPS and HDR** setting
- Optionally becomes the **primary** display, or the **only** active display (`--exclusive`)
- Per-client monitor identity, so Windows remembers scaling and other settings per client
- Automatic cleanup when the stream ends, when Sunshine exits, or when `vdisplay` crashes
- Bundled SudoVDA driver with one-command install/uninstall
- Single self-contained `.exe`, no admin rights needed for streaming

## Requirements

- Windows 10/11 x64
- Sunshine
- The SudoVDA driver — if Apollo is installed, it is already there

## Quick start

1. **Download** `vdisplay.exe` from the [Releases](../../releases/latest) page and put it in a folder your user can write to, e.g. `C:\Tools\` (the log file `vdisplay.log` is written next to the exe, or to `%TEMP%` if that fails).

2. **Install the driver** (once; shows a UAC prompt):

   ```
   C:\Tools\vdisplay.exe install-driver
   ```

3. **Configure Sunshine** (Web UI → Configuration):
   - *Audio/Video → Display Device Id* (`output_name`): **leave empty**, so Sunshine captures the primary display.
   - *Audio/Video → Device configuration* (`dd_configuration_option`): **Disabled**. Sunshine applies it before prep commands run, when the virtual display doesn't exist yet.
   - *General → Command Preparations*, add (globally or per application):

     | Field | Value |
     |---|---|
     | Do   | `C:\Tools\vdisplay.exe start --primary` |
     | Undo | `C:\Tools\vdisplay.exe stop` |

     Keep *Run as administrator* the same for both commands.

   Or the equivalent in `sunshine.conf`:

   ```
   global_prep_cmd = [{"do":"C:\\Tools\\vdisplay.exe start --primary","undo":"C:\\Tools\\vdisplay.exe stop","elevated":false}]
   ```

4. Connect with Moonlight. A virtual display with the client's resolution/FPS/HDR appears and becomes the primary display; it is removed when you end the stream.

Use `--exclusive` instead of `--primary` to turn off all physical displays while streaming. They are restored afterwards.

## Commands

```
vdisplay start [options]   Create the virtual display (Sunshine "do" command)
vdisplay stop              Remove it again (Sunshine "undo" command)
vdisplay status            Show version, whether a display is active, driver state and log path
vdisplay install-driver    (Re)install the bundled SudoVDA driver (asks for admin rights)
vdisplay uninstall-driver  Remove the SudoVDA device, driver package and certificate (asks for admin rights)
vdisplay version           Print the version
```

### `start` options

Defaults are taken from the `SUNSHINE_CLIENT_*` environment variables that Sunshine sets for prep commands, so `start --primary` alone is usually all you need.

| Option | Description | Default |
|---|---|---|
| `--width <px>` | Display width | `SUNSHINE_CLIENT_WIDTH`, else 1920 |
| `--height <px>` | Display height | `SUNSHINE_CLIENT_HEIGHT`, else 1080 |
| `--fps <hz>` | Refresh rate, e.g. `60` or `59.94` | `SUNSHINE_CLIENT_FPS`, else 60 |
| `--hdr [true\|false]`, `--no-hdr` | Force HDR on/off | `SUNSHINE_CLIENT_HDR`, else unchanged |
| `--primary` | Make the virtual display the primary display | off |
| `--exclusive` | Disable all other displays while streaming (implies `--primary`) | off |
| `--name <text>` | Monitor name / identity | `SUNSHINE_CLIENT_NAME` |
| `--id <text>` | Stable identity used to derive the monitor GUID | `--name` |
| `--adapter <gpu>` | Render GPU by name, e.g. `"NVIDIA GeForce RTX 4090"` | system default |
| `--no-watch` | Don't remove the display automatically when `sunshine.exe` exits | off |

Passing the values explicitly is equivalent:

```
cmd /C "C:\Tools\vdisplay.exe start --width %SUNSHINE_CLIENT_WIDTH% --height %SUNSHINE_CLIENT_HEIGHT% --fps %SUNSHINE_CLIENT_FPS% --hdr %SUNSHINE_CLIENT_HDR% --name "%SUNSHINE_CLIENT_NAME%" --primary"
```

## How it works

SudoVDA has a watchdog: if it receives no request for its timeout (3 s by default), it removes **all** virtual displays. Therefore:

- `vdisplay start` launches a detached background daemon. The daemon creates the display, applies mode / primary / HDR settings and keeps pinging the driver. `start` returns as soon as the display is ready, so Sunshine can continue starting the stream.
- `vdisplay stop` signals the daemon through a named event; the daemon removes the display and restores the previous display layout.
- If Sunshine exits or the daemon is killed, the pings stop and the driver removes the display. Windows then restores the previous layout from its display configuration database — this also brings back physical screens turned off by `--exclusive`.

The watchdog is global to the driver, so pings from **any** client keep displays alive. If Apollo is running at the same time and the daemon is killed, the leftover display is not removed by the watchdog; the next `vdisplay stop` or `start` cleans it up using the GUID recorded in `vdisplay.state`.

The monitor GUID is derived from the client name, so Windows remembers per-client settings such as scaling. (Apollo uses the client UUID, which Sunshine does not expose.)

## Driver

`drivers/sudovda/` contains the SudoVDA driver package shipped with Apollo (`SudoVDA.inf/.dll/.cat/.cer`); it is embedded into the exe at build time. `install-driver` does what Apollo's `install.bat` does, using SetupAPI directly instead of `nefconc.exe`:

1. Import `sudovda.cer` into the local machine `Root` and `TrustedPublisher` certificate stores.
2. Create the `Root\SudoMaker\SudoVDA` device node (Display class) if missing.
3. Install `SudoVDA.inf` on that node with `UpdateDriverForPlugAndPlayDevices`.

`start` never installs the driver; it fails with an error if the driver is missing, so streaming never needs admin rights.

`uninstall-driver` stops any running virtual display, removes the device node, deletes the driver package (`oemNN.inf`) from the driver store and removes the certificates. **Apollo uses the same driver**, so its virtual displays stop working too.

## Troubleshooting

- Run `vdisplay status` to check the version, driver state and the log file location.
- Every run is logged to `vdisplay.log`; include it when reporting issues.
- `scripts/test-exclusive.ps1` checks that `--exclusive` turns off and restores the physical displays (falling back to `DisplaySwitch.exe /extend` if restoring fails).

## Building

Requires a Rust toolchain (edition 2024, Rust 1.85+) with the MSVC target.

```sh
cargo build --release
# output: target/release/vdisplay.exe
```

The version reported by `vdisplay version` comes from the `VDISPLAY_VERSION` environment variable at build time, falling back to the version in `Cargo.toml`:

```sh
VDISPLAY_VERSION=1.2.0 cargo build --release
```

### Releases

Releases are built by GitHub Actions ([`.github/workflows/release.yml`](.github/workflows/release.yml)). Either:

- push a tag such as `v1.2.0`, or
- run the **Release** workflow manually from the Actions tab and enter the version (e.g. `1.2.0`); the tag `v1.2.0` is created on the selected branch.

The workflow builds `vdisplay.exe` with that version embedded and publishes `vdisplay-v<version>-windows-x64.zip` plus its SHA-256 checksum to a GitHub release. Versions with a suffix (e.g. `1.2.0-beta.1`) are marked as pre-releases.

## License

[GPL-3.0](LICENSE).

The bundled SudoVDA driver is developed by [SudoMaker](https://github.com/SudoMaker/SudoVDA) and is redistributed as shipped with [Apollo](https://github.com/ClassicOldSong/Apollo).
