# Changelogs

Release history for Subaroll. Public releases use the `major.minor` format (`v1.0`, `v1.1`, `v1.2`).

## v1.2

**Summary:** Hardened the app's web content policy, serialized preview work, made export job tracking atomic, capped media-tool output and subtitle/font resource use, and pinned release workflow actions.

- Enabled a restrictive Content Security Policy for the bundled WebView, limiting scripts and connections to app-owned resources and Tauri IPC while preserving the app's dynamic preview styles.
- Serialized preview rendering so rapid seeking or repeated adjustments keep at most one render in flight and only queue the latest requested preview. Preview frames have a bounded size.
- Made export and compression jobs reserve a single shared job slot before background work begins. This prevents overlapping runs and keeps cancellation tied to the active process.
- Bounded FFmpeg/FFprobe output captured in memory and capped subtitle text and adjacent-font files processed during inspection, preview, and export. Font symlinks are skipped.
- Pinned GitHub Actions to immutable commit SHAs, disabled checkout credential persistence, and scoped the release write permission to the publishing job.
- Kept public release tags and download naming in `major.minor` form. This release is `v1.2`; the app uses an internal SemVer version for update comparisons.
- Follow-up within the same `v1.2` release: Windows GPU and preset controls now remain visible but disabled and clearly marked Windows-only on macOS. The workspace now respects the window height and preserves a 28px bottom inset, using the previous excess top inset. Internal updater version is now `1.2.2`; public release tag and download naming stay `v1.2`.
- Additional follow-up within `v1.2`: the Discord size-target heading now wraps instead of colliding with the size controls at narrow window widths. Removed the duplicate compression attribution from the app UI; project credit remains in the GitHub repository. Removed Tauri's legacy Carbon-required flag and explicitly declared the macOS app principal for newer macOS launch compatibility. Internal updater version is `1.2.3`.
- Additional follow-up within `v1.2`: the updater button and status messages now show the public `major.minor` version (for example, `v1.2`) while retaining internal patch versions for update ordering. Internal updater version is `1.2.4`.

## v1.1

**Summary:** Better bitrate-to-resolution and frame-rate planning for target-size compression; Windows AV1/NVIDIA/AMD/Intel encoder options; Subaroll name and CC icon; more reliable native release builds; and signed in-app updates for macOS and Windows. All updates previously published separately as v1.2 are included in this consolidated v1.1 release.

### Consolidated release packaging

- Updated the existing `v1.1` release with the current macOS ARM64 and Windows 11 x64 builds instead of creating a new release.
- Kept the public release tag and download filenames in `major.minor` form, such as `v1.1` and `Subaroll_1.1_aarch64.dmg`.
- Added signed updater bundles and signatures, plus `latest.json` entries for macOS ARM64 and Windows x64.
- Removed the separate v1.2 release and tag after verifying the consolidated v1.1 assets and updater manifest.

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
- Added signed in-app updates for macOS and Windows. Subaroll checks GitHub Releases on launch, verifies updates with its embedded public key, and can install an available update from the app. Windows installs updates in passive mode. The earlier v1.1 installers need a one-time manual replacement with this updater-enabled build.
- Added macOS first-launch guidance for the unsigned, non-notarized app, including the app-specific quarantine-removal command.
- Published signed updater packages and a `latest.json` manifest for Apple silicon macOS and Windows 11 x64. Release and download names use `major.minor`; updater metadata retains the valid internal SemVer needed for version comparisons.

## v1.0

### Initial native release

- Introduced the native Rust/Tauri desktop app for macOS and Windows.
- Added external subtitle burn-in for SRT, ASS, SSA, and VTT files, with preview, timing adjustment, position and size controls, and a short test export.
- Used FFmpeg and libass to render subtitles into the video while retaining the source frame dimensions and aspect ratio for burn-in exports, with HDR and 10-bit preservation dependent on the selected encoder and format.
- Added target-file-size video compression with automatic bitrate budgeting, resolution planning, and output export.
- Added encoding-quality controls and platform-specific encoding paths.
