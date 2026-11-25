# Changelogs

Release history for Subaroll. Public releases use the `major.minor` format (`v1.0`, `v1.1`, `v1.2`).

## Unreleased

**Summary:** Landscape layout with notched pill sliders, automatic subtitle matching, and a reworked Compress mode with a strict size ceiling. Nothing here has run on a real Mac yet.

### Layout and controls
- Fixed-size window (880x764, 792 on macOS): a 16:9 preview with a three-column dock under it. Encoders are on the left, sliders in the middle, files, the Burn/Compress switch and actions on the right, and Check for updates sits in the bottom-left of the dock. The window can't be resized or maximized, and it always starts in Burn mode.
- Pill sliders with a dotted fill and a capsule thumb. They slide continuously. Landmarks (Middle, Original, Recommended) act as a soft detent over a fixed pixel zone that turns the track blue.
- Burn mode no longer has the CTT/Balanced/Fast preset dropdown. Encoders always use their best preset, and CQ is the only quality control.
- Picking a video loads a subtitle file from the same folder, or a `Subs`/`Subtitles` folder, when the name or the show, season and episode match (`find_subtitle`). A subtitle you chose yourself is never replaced.

### Compress mode
- Maximum target is 250 MB. The size you pick is a ceiling and is never exceeded. From 5 MB up, 1 MB means 1,000,000 bytes minus 1%. Below 5 MB it stays 1,048,576 bytes.
- Targets that leave under 150 kbps of video are refused up front, with the smallest size that works.
- A live estimate (resolution, frame rate, video bitrate) comes from the same plan the export uses (`plan_compression`).
- Encoder choice (NVIDIA, Intel, AMD, CPU, and Apple VideoToolbox on macOS) and a codec row (Auto, AV1, H.264). The CPU option is x264 (H.264) two-pass.
- On Windows, below 1 Mbps of video every choice goes straight to x264 two-pass, because the hardware encoders cannot hold a size that small. On macOS VideoToolbox is tried first.
- NVIDIA constant-QP compression searches for the largest file that fits (up to 4 attempts) instead of a single correction. If no setting fits, it falls back to bitrate control, then to x264. Bitrate-controlled encoders get one corrective re-encode, then x264.
- Thin 60 fps H.264 budgets (about 2.4 to 3.2 Mbps) now plan 720p60 instead of 900p60, which measured cleaner at the same size.
- 30-second test button in Compress mode, with the size budget scaled to the clip.
- The video bitrate is capped at 1.5x the source's so tiny sources are not bloated.

### Development
- `npm start` works on Node 22 on Windows. `cargo test` runs the unit tests.
- Real-ffmpeg checks are ignored by default: `cargo test real_compression -- --ignored --nocapture`. The ratio test reads `SUBAROLL_TEST_SOURCE`, `SUBAROLL_TEST_OUT`, and optionally `SUBAROLL_TEST_MB`, `SUBAROLL_TEST_RATIO` and `SUBAROLL_TEST_ENCODER`.
- The whole compression job is now `compress_video`, independent of the Tauri window, so it can be tested from a terminal.

## v1.2

**Summary:** Settings restore correctly on restart, controls fit better in the app window, and local macOS updates restart the installed app more reliably. v1.2 also hardens the WebView and limits resource use during preview and export.

- Enabled a restrictive Content Security Policy for the bundled WebView, limiting scripts and connections to app-owned resources and Tauri IPC while preserving the app's dynamic preview styles.
- Serialized preview rendering so rapid seeking or repeated adjustments keep at most one render in flight and only queue the latest requested preview. Preview frames have a bounded size.
- Made export and compression jobs reserve a single shared job slot before background work begins. This prevents overlapping runs and keeps cancellation tied to the active process.
- Bounded FFmpeg/FFprobe output captured in memory and capped subtitle text and adjacent-font files processed during inspection, preview, and export. Font symlinks are skipped.
- Pinned GitHub Actions to immutable commit SHAs, disabled checkout credential persistence, and scoped the release write permission to the publishing job.
- Kept public release tags and download naming in `major.minor` form. This release is `v1.2`; the app uses an internal SemVer version for update comparisons.
- On macOS, the Windows GPU and preset controls are disabled and labeled Windows only. Adjusted window and controls panel spacing to give the title bar buttons and bottom status row room.
- The compression size heading wraps at narrow widths. Compression credit is listed in the repository.
- Updated macOS bundle launch metadata. The local install script now waits for the old app to exit, builds the local bundle without updater artifacts, and opens the installed bundle through macOS Launch Services.
- Subtitle delay, position, size, and quality restore on restart. Fixed a bug that subtracted five points from a saved subtitle position and prevented older settings from being migrated repeatedly.
- The app remembers the last selected mode, compression target and codec, and Windows encoder and preset. GPU detection can use Auto when a saved encoder is unavailable without overwriting the saved preference.
- The updater displays the public version, such as `v1.2`. This build uses internal version `1.2.9` so existing installations can detect the update.

- Windows release builds open directly into the Subaroll window. FFmpeg and FFprobe run in the background without console windows. The EXE and MSI include both media tools and an offline WebView2 installer for systems missing the runtime.
- On Windows, the settings panel fits its contents when resized or maximized. The footer stays with the action buttons, the sidebar keeps a stable width, and a smaller top inset fits the native Windows title bar. Compact controls keep export progress visible at short window heights.
- Windows GPU detection uses a 256×256 test encode. This fixes false negatives when NVIDIA NVENC rejects the previous 64×64 input. Hardware options still require a successful encoder test.
- Update checks ask caches to revalidate GitHub’s manifest and include the installed build in the request. Hovering over the update button shows the installed and available build numbers.
- The Windows Auto encoder option names the selected GPU or CPU encoder and its codec. The label reflects whether the source needs H.264 or 10-bit HEVC.
- The preview scales to the available window space while preserving its aspect ratio. The settings panel fits its contents, and the action buttons, status, and update control remain visible at short window heights.

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
