# Subaroll

Subaroll burns external subtitle files into video or compresses video to a chosen file-size target. It is a native Rust/Tauri desktop app for Apple-silicon Macs and Windows 11, using FFmpeg and libass for video and subtitle rendering.

## Features

### Subtitle burn-in

- Supports SRT, ASS, SSA, VTT, SUB, and SMI subtitle files.
- Preserves authored ASS styling, including fonts, colors, outlines, shadows, positioning, and signs where supported. Size and position controls adjust dialogue while keeping authored signs in place.
- Plain-text subtitles use a consistent white, outlined preset; subtitle delay changes cue timing without shifting the video or audio.
- Scrub a preview, adjust subtitle position against the full video frame, and render a short test before exporting.
- Cinematic sources receive bottom padding when needed, so subtitles can be positioned in the padded frame without stretching the picture.
- Burn-in exports preserve the source's resolution, aspect ratio, frame timing, and HDR/10-bit signaling where the selected format and encoder support them.

### File-size compression

- Set a target size; Subaroll calculates a bitrate budget and chooses a suitable output resolution and frame rate without upscaling.
- Resolution is retained when the budget can support it. Higher frame rates are kept when the bitrate budget allows; frame rate is reduced only when required by a constrained target.
- On Windows, Auto prefers a usable hardware AV1 encoder, then falls back to compatible hardware encoding or CPU encoding. NVIDIA NVENC, Intel Quick Sync, and AMD AMF options are detected and checked before use.
- NVIDIA compression uses CQP 20 with a calculated correction pass when necessary to meet the size target. CPU encoding uses a slower two-pass mode for tighter size targeting.
- On macOS, subtitle burn-in uses VideoToolbox hardware encoding. Compression uses FFmpeg's CPU encoder.

## Install 1.2

- **macOS 11 or newer, Apple silicon only (M1 or newer):** download [Subaroll 1.2 for macOS](https://github.com/AbrahamYB/Subaroll/releases/download/v1.2/Subaroll_1.2.0_aarch64.dmg), drag Subaroll into Applications, then right-click the app and choose **Open** on first launch. Install FFmpeg once with `brew install ffmpeg-full`.
- **Windows 11 x64:** download the [setup EXE](https://github.com/AbrahamYB/Subaroll/releases/download/v1.2/Subaroll_1.2.0_x64-setup.exe) (recommended) or [MSI installer](https://github.com/AbrahamYB/Subaroll/releases/download/v1.2/Subaroll_1.2.0_x64_en-US.msi). FFmpeg and FFprobe are included. If SmartScreen appears, choose **More info → Run anyway**.

The release installers are unsigned and not notarized. Windows release packages include the pinned FFmpeg build documented in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## App updates

Subaroll 1.2 checks GitHub Releases when it opens. When a signed update is available, select **Install v…** in the status row; the app downloads and installs it, then restarts. Windows installs the update in passive mode. Updates are verified with Subaroll's embedded public key before installation. The v1.1 installers predate the updater and need to be replaced once with v1.2; subsequent releases can be installed in the app.

### macOS first launch

If macOS still blocks the unsigned, non-notarized app after moving it to Applications, open Terminal and run:

```sh
xattr -dr com.apple.quarantine "/Applications/Subaroll.app"
```

Then open Subaroll from Applications. This removes the quarantine attribute from that app bundle only.

## Run from source

1. Install Rust with `rustup`, Node.js, and a full FFmpeg build that includes libass.
2. Run `npm install` once.
3. Run `npm start`.

On macOS, Subaroll automatically prefers Homebrew's `ffmpeg-full`. If FFmpeg is installed elsewhere, choose its executable from the app.

## Build

- macOS: `npm run build:mac`
- Windows, run on Windows: `npm run build:win`
- Run automated checks: `npm test`

Release builds are produced natively by GitHub Actions: ARM64 on macOS and x64 on Windows. Subaroll is built with Rust and Tauri; releases do not include Electron.

## Changelogs

See [changelogs.md](changelogs.md) for the full release history and version-by-version details.

## Credits and license

The target-size compression feature is independently written in Rust, using the workflow and design of [vladaad's DiscordCompressor](https://github.com/vladaad/discordcompressor) as its reference. Full credit to vladaad for the original compression approach. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for bundled component licenses.

Subaroll's code is released under the MIT License. Third-party components keep their own licenses.
