# Changelogs

Release history for Subaroll. Versions are shown using the public release names (`v1.0`, `v1.1`, `v1.2`).

## v1.2

### In-app updates

- Added update checks when Subaroll opens on macOS and Windows.
- Added an in-app action to download, verify, and install an available update, then restart the app.
- Update packages are verified using Subaroll's embedded public key before installation.
- Windows updates install in passive mode.
- Published signed updater bundles and a `latest.json` manifest for macOS ARM64 and Windows 11 x64.
- The v1.1 app must be replaced manually once with v1.2 because it predates the updater. Later updates can be installed from inside the app.

### macOS first launch

- Added guidance for opening the unsigned, non-notarized macOS build after moving it to Applications, including an app-specific quarantine-removal command for users who choose to use it.

### Release pipeline

- Kept the public release name and tag in the short `v1.2` format while using valid `1.2.0` SemVer metadata inside the app and its installer filenames.
- Built the macOS release for Apple silicon and the Windows release for Windows 11 x64.
- Included the macOS updater app archive alongside the DMG and Windows updater signatures alongside the installers.

## v1.1

### Compression behavior

- Refined target-size planning so the encoder makes better use of the available bitrate before reducing picture resolution.
- Avoided dropping a source that can fit at 1080p to 900p prematurely.
- Made the planner account for frame rate as well as resolution and bitrate. It keeps 60 fps when the requested size can support the chosen resolution and frame rate, lowering frame rate only when the size budget requires it.
- Kept the no-upscaling behavior for sources below the selected output resolution.
- Added Windows hardware-encoder selection, including automatic hardware AV1 preference when a usable AV1 encoder is detected, plus NVIDIA NVENC, Intel Quick Sync, and AMD AMF options when available.
- Used NVIDIA CQP 20 with a correction pass when needed to meet the requested target size; CPU compression uses two-pass encoding for tighter size targeting.
- Kept macOS subtitle burn-in on VideoToolbox hardware encoding and macOS target-size compression on FFmpeg's CPU encoder.

### App and release

- Renamed the product to Subaroll across the app, macOS and Windows product metadata, release names, and repository copy.
- Updated the app icon to the current CC design.
- Preserved the macOS bundle identifier so existing app preferences remain available after upgrading.
- Added native release builds for Apple silicon macOS and Windows 11 x64, with automated release checks for the platform-specific build paths.
- Improved the test runner so reported failures fail the release checks, and made Cargo discovery use the configured Rust toolchain.
- Documented the independent Rust compression implementation's relationship to vladaad's DiscordCompressor. The project credits its compression workflow and design as the reference; it does not embed the upstream compressor binary.

## v1.0

### Initial native release

- Introduced the native Rust/Tauri desktop app for macOS and Windows.
- Added external subtitle burn-in for SRT, ASS, SSA, and VTT files, with preview, timing adjustment, position and size controls, and a short test export.
- Used FFmpeg and libass to render subtitles into the video while retaining the source frame dimensions and aspect ratio for burn-in exports, with HDR and 10-bit preservation dependent on the selected encoder and format.
- Added target-file-size video compression with automatic bitrate budgeting, resolution planning, and output export.
- Added encoding-quality controls and platform-specific encoding paths.
