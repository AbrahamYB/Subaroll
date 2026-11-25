# Changelogs

Release history for Subaroll. Public releases use the `major.minor` format (`v1.0`, `v1.1`, `v1.2`).

## v1.3

Subaroll v1.3 brings a redesigned workspace, automatic subtitle matching, and rebuilt target-size compression to Apple silicon Macs and Windows 11 x64.

### Original redesign and compression rework

- Rebuilt the workspace around a fixed-size 16:9 preview with a three-column dock for encoders, sliders, file selection, and export actions. Burn and Compress switch within the same dock, the app starts in Burn mode, and Check for updates sits below the encoder list.
- Introduced continuous pill sliders with soft snap points. Subtitle size now goes up to 200%. Burn uses the encoder's quality preset automatically, with CQ as the quality control.
- Added offline subtitle matching beside the video and in nearby `Subs` or `Subtitles` folders. A matching filename takes priority, including names with a language suffix. The app can also match by series, season, and episode. A manually selected subtitle stays selected.
- Added an editable compression target up to 250 MB, live estimates for resolution, frame rate, and video bitrate, and a 30-second test that uses the full job's planned bitrate.
- Added encoder selection and Auto, AV1, and H.264 codec choices to the new Compress interface. AV1 requires compatible hardware. The planner favors 720p over 900p for 60 fps H.264 footage at roughly 2.4–3.2 Mbps.
- Fixed `npm start` on Windows with Node 22.

#### Compression fixes

- Fixed compression landing far below the size you picked. With NVIDIA, a single correction pass left test clips at about half the target (10.4% of the source instead of 20%). The app now tries up to four quality settings and keeps the largest file that fits.
- The size you pick is now a strict ceiling. From **5 MB up**, the app uses 1,000,000 bytes per MB and reserves 1% so the file fits services that count that way. **Below 5 MB**, the app keeps the original 1,048,576 bytes per MB and applies no margin. For example, a 4 MB target allows up to 4,194,304 bytes.
- If a size can't be reached, the app says “Couldn't fit this video under X MB. Choose a larger size.” and deletes the partial file.
- Very small sizes go straight to the CPU encoder on Windows. When the planned video bitrate is under 1 Mbps, every encoder choice uses two-pass x264. In testing, NVIDIA could not fit 20 MB for a five-minute or ten-minute 1080p60 video (0.44 and 0.19 Mbps), and Intel Quick Sync could not fit the ten-minute one.
- If a bitrate-controlled hardware encode comes out over the target, the app re-encodes once at a lower bitrate and switches to two-pass x264 if it still can't fit. NVIDIA tries bitrate control after its quality-setting search.
- Sizes that would leave under 150 kbps of video are refused before encoding starts, with the smallest size that works. The planner limits video bitrate to 1.5× the source bitrate when known, subject to the video bitrate floor.
- On macOS, Compress mode now uses Apple VideoToolbox, with the CPU encoder as the fallback. Both paths were verified on Apple silicon during follow-up testing.

### Follow-up fixes after the handoff

#### Window and controls

- On Mac, the window opens centered in the usable space between the menu bar and Dock. On shorter displays, the preview gives up height so the controls fit. The title-bar strip can be dragged to move the window.
- Both modes on Mac let you select Apple VideoToolbox or CPU. Auto prefers VideoToolbox when available, using HEVC for Burn and H.264 for Compress. The encoder choices appear in the same order on Mac and Windows, with unsupported choices visible and disabled.
- Slider snap points turn blue immediately when reached, including for a saved or default value on launch. Export and 30-second test buttons stay enabled during slider adjustments once the required files are ready.

#### Preview and subtitles

- Selecting a video shows a seekable preview in both modes. Compress shows the source frame, and Burn can show the video before a subtitle file is selected. A subtitle preview error leaves the video visible.
- Burn uses the same libass rendering as export for delay, position, and size. Authored ASS positions and movement stay in place, including when a positioned cue shares its style with ordinary dialogue.
- Seeking starts rendering immediately. Rapid changes keep only the latest pending preview, FFmpeg detection is reused, and frames are resized after rendering to load faster. Replacing a preview releases the previous image.

#### Compression and export reliability

- The Compress button now sends the selected file-size target to the encoder. The export checks that target when you click, even while the live estimates are still updating.
- Resolution and bitrate estimates update while dragging the target-size slider and reflect the source-based bitrate cap. The FPS display stays tied to the source, and compression preserves frame timing without dropping frames to meet the target.
- Burn and Compress clear progress after an export and reuse the current preview when its settings are unchanged. Cancellation stops the active encoder and prevents later passes from starting. macOS releases its keep-awake helper after each pass.
- An export is saved to its final path after validation. Cancelled or failed jobs clean up temporary files, and any previous file at the destination is preserved.

#### Release builds

- Published an Apple silicon DMG and Windows 11 x64 setup EXE and MSI, with signed in-app updates for both platforms. The Windows installer checks verify that the GUI app, FFmpeg, and FFprobe are included.

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
