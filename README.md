# Subaroll

Subaroll burns subtitles into video and compresses video to a chosen file-size target. It is a Rust/Tauri desktop app for Apple-silicon Macs and Windows 11, using FFmpeg and libass for video and subtitle rendering.

## Features

### Subtitle burn-in

- Open SRT, ASS, SSA, VTT, SUB, and SMI subtitle files, or select a supported embedded text subtitle track. Image-based subtitle tracks are not supported by the language picker.
- Matching subtitle files are found beside the video and in nearby `Subs` or `Subtitles` folders. When multiple supported languages are detected, the language picker shows the available tracks.
- Preview the rendered subtitles while adjusting delay, position, and size. A 30-second test exports the whole clip when it is shorter than 30 seconds.
- Preserve authored ASS styling and adjust dialogue while keeping signs in place where supported. Plain-text subtitles use a white, outlined preset.
- Preserve source resolution, aspect ratio, frame timing, and HDR/10-bit signaling where supported. Wide cinematic sources can receive lower padding for subtitles without stretching the picture.
- Select an available encoder or use Auto. Burn quality shows the selected encoder's actual setting: NVIDIA CQP, AMD CQP, Intel ICQ, CPU CRF, or Apple VideoToolbox quality. NVIDIA and AMD CQP, Intel ICQ, and CPU CRF reach 10; AMD displays the P-frame QP, with offsets for other frame types.

### File-size compression

- Choose a target from **1 to 750 MB**, with snap points at 25, 50, 100, and 250 MB. The app estimates resolution and video bitrate as the target changes.
- Compression preserves source frame timing and chooses resolution according to the bitrate budget without upscaling. The 30-second test uses the planned settings for the full video.
- Choose Auto, H.264, or hardware AV1 when supported. On Windows, available NVIDIA NVENC, Intel Quick Sync, and AMD AMF encoders are detected; on macOS, compression can use Apple VideoToolbox or CPU encoding.
- The selected size is a ceiling. Targets of 5 MB and above use decimal MB with a 1% margin; smaller targets use 1,048,576 bytes per MB. Targets that cannot provide at least 150 kbps of video are refused before encoding.
- NVIDIA can try multiple CQP values for short exports; long exports use bitrate control to avoid repeated full-length passes. Hardware encoding can retry or fall back to two-pass CPU encoding when needed to meet the target.

## Install 1.3

- **macOS 11 or newer, Apple silicon only (M1 or newer):** download [Subaroll 1.3 for macOS](https://github.com/AbrahamYB/Subaroll/releases/download/v1.3/Subaroll_1.3_aarch64.dmg), drag Subaroll into Applications, then right-click the app and choose **Open** on first launch. Install a full FFmpeg build with libass, such as Homebrew's `ffmpeg-full`.
- **Windows 11 x64:** download the [setup EXE](https://github.com/AbrahamYB/Subaroll/releases/download/v1.3/Subaroll_1.3_x64-setup.exe) or [MSI installer](https://github.com/AbrahamYB/Subaroll/releases/download/v1.3/Subaroll_1.3_x64.msi). FFmpeg and FFprobe are included. If SmartScreen appears, choose **More info → Run anyway**.

The release installers are unsigned and not notarized. Windows release packages include the pinned FFmpeg build documented in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## App updates

Subaroll checks GitHub Releases when it opens. When an update is available, select **Install v…**; the app downloads and verifies the signed update, installs it, then restarts. Windows installs updates in passive mode. Builds without the updater require a one-time manual installation of an updater-enabled version.

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

On macOS, Subaroll automatically prefers Homebrew's `ffmpeg-full`.

## Build

- macOS: `npm run build:mac`
- Windows, run on Windows: `npm run build:win`
- Run automated checks: `npm test`

Release builds are produced natively by GitHub Actions: ARM64 on macOS and x64 on Windows.

## Changelogs

See [changelogs.md](changelogs.md) for the full release history and version-by-version details.

## Credits and license

The target-size compression feature is independently written in Rust, using the workflow and design of [vladaad's DiscordCompressor](https://github.com/vladaad/discordcompressor) as its reference. Full credit to vladaad for the original compression approach. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) for bundled component licenses.

Subaroll's code is released under the MIT License. Third-party components keep their own licenses.
