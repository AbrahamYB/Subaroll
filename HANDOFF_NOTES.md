# Handoff notes: UI redesign and Compress rework

Branch: `ui-redesign-and-compress-rework` (one commit on top of `main` at `4b44284`).
This file records what was done, what stuck, what was tried and dropped, the measured results, and what is still unverified. It was written from the working session that produced this branch. `changelogs.md` has a shorter "Unreleased" entry.

## 1. Run it

- `npm install`, then `npm start` (Windows needs `shell: true` for `tauri.cmd` on Node 22; that fix is in `scripts/run-tauri.js`).
- Do not launch `src-tauri/target/debug/subaroll.exe` directly. Without the dev wrapper the WebView shows "can't reach this page".
- `npm test` / `cargo test --manifest-path src-tauri/Cargo.toml`: 27 unit tests pass, 5 more are `#[ignore]`d.
- Real-ffmpeg checks (need `ffmpeg`/`ffprobe` on `PATH`, take minutes):
  - `cargo test real_compression_meets -- --ignored --nocapture` (synthetic 40 s clip: size ceilings, refusal, clip test).
  - `cargo test real_compression_to_fraction -- --ignored --nocapture` with env `SUBAROLL_TEST_SOURCE`, `SUBAROLL_TEST_OUT`, and either `SUBAROLL_TEST_RATIO` (default 0.2) or `SUBAROLL_TEST_MB`; optional `SUBAROLL_TEST_ENCODER` (`auto`, `nvenc`, `qsv`, `amf`, `videotoolbox`, `software`). It prints source, plan, encoder, output and time.
- Test through `compress_video`, not by driving the window. An early attempt to automate the UI with mouse and key scripts sent input to the wrong window; do not repeat it.
- Do not commit npm lockfile churn (a local `npm install` removes the `libc` fields from `package-lock.json`; that diff was reverted).

## 2. Design rules that stuck (the user's decisions)

- Flat surfaces. No background gradients, glows or heavy shadows. The only decoration left is on the sliders (dotted fill, soft thumb glow).
- Landscape window. The 16:9 preview is the priority and must stay 16:9. The window is fixed size and cannot be resized or maximized.
- No "drop first" screens or any gating screen. Keep changes incremental from the two-panel layout.
- Sliders are continuous. Landmarks are a soft detent, not preset buttons.
- Encoder list is a vertical chip list. Check for updates lives in the bottom-left corner of the dock.
- The app always starts in Burn mode (the last mode is no longer remembered).
- Idle status text ("ready", "choose a video…") was removed. Status shows only as a small bar over the top of the preview when there is something to say.

## 3. Final layout

Window 880x764 (macOS 792, for the overlay title bar). `src-tauri/src/lib.rs` also forces size, un-maximizes and centers at startup because the window opened oversized on one Windows setup.

- Top: 16:9 preview (`.preview-slot`), seek bar.
- Dock, three columns:
  - Left: encoder chips (Auto, NVIDIA, Intel, AMD, CPU; on macOS Auto, Apple, CPU), "Check for updates" at the bottom.
  - Middle: Burn mode shows Delay and Position (left), Size and Quality (right). Compress mode shows Target size, estimate tiles (resolution, frame rate, video bitrate) and a codec row.
  - Right: video and subtitle pickers, Burn/Compress switch (vertically centered, equal gaps), then 30s test and the main action button.
- Sliders: pill track with dotted fill, capsule thumb, end labels and landmark labels. Landmarks (Middle for position, Original for size, Recommended for quality, 0 for delay) soft-lock inside a fixed 9 px zone on the bar (`ZONE_PX` in `renderer.js`). The mapping is continuous (a plateau around the landmark), so the thumb never jumps and there is no direction state. The track turns blue and the thumb turns solid blue while on a landmark, with a ~0.28 s fade. Detents never swallow an end of the slider.
- Ranges: delay -5..5 s (step 0.05), position 0..100, size 50..200 %, quality (CQ) 60..100, target size 1..250 MB.
- The Burn/Compress switch cross-fades the two control sets inside the same dock, so nothing changes size.

## 4. Features that stuck

### Subtitle auto-match (`find_subtitle` in `media.rs`)
When a video is picked, look in its folder and in `Subs` / `Subtitles` for a subtitle with the same name (a language suffix like `.en` still counts), else the same series, season and episode (`S02E03`, `S2E3`, `1x03`, `- 03`, `EP03`). Season must match when the video has one; series names must roughly match. Ranking: exact name, then ASS before SRT etc., then alphabetical. A subtitle the user picked by hand is never replaced; one chosen automatically is replaced when the next video is loaded. Offline and read-only.

### Burn mode
- The CTT / Balanced / Fast preset dropdown was removed. The encoders always use their best preset (`windowsPreset` is sent as `'quality'`).
- Open point: on Windows the CQ slider is still converted with a CTT-derived formula (`cq = 40 - quality * 0.244`, in `windows_encoder_args`). It is not passed straight to the encoder. The user's remark about "CTT overriding CQ 90" may have meant that conversion, or may have meant "improve the dropdown" rather than remove it. Unresolved.

### Compress mode
- Maximum target 250 MB (`MAX_COMPRESSION_MB`).
- The picked size is a strict ceiling (`ceiling_bytes`): from 5 MB up, 1 MB = 1,000,000 bytes minus 1 %; below 5 MB, 1 MB = 1,048,576 bytes and no margin. The old behavior accepted up to +3 %.
- Targets that leave under 150 kbps of video (`MIN_VIDEO_BITRATE`) are refused before encoding, with the smallest size that works (`check_compression_target`, `min_target_mb`). This is a bitrate floor only; the app cannot judge picture quality.
- Live estimate: the `plan_compression` command returns resolution, frame rate, video bitrate and whether the CPU encoder will be used. The same plan drives the export.
- Planner (`compression_plan_bytes`, `compression_geometry`): budget is 96 % of the ceiling (`SIZE_BUDGET`); video bitrate is capped at 1.5x the source's bitrate (and the size goal shrinks with it) so small sources are not bloated; resolution tiers come from bitrate. New: for H.264 at above 30 fps with a thin budget (about 2.4 to 3.2 Mbps), plan 720p60 instead of 900p60. AV1 and 30 fps tiers are unchanged. Frame rate is never reduced by this rule.
- Encoder choice (`encoder_preference`: `auto`, `nvenc`, `qsv`, `amf`, `videotoolbox`, `software`) and codec row (Auto, AV1, H.264). CPU is x264 (H.264) two-pass. AV1 is hardware only.
- Encoder strategy (`compress_video`):
  - Windows, video bitrate under 1 Mbps (`HARDWARE_MIN_VIDEO_BITRATE`): every encoder choice goes straight to x264 two-pass (`use_cpu_encoder`). The UI shows "Under 1 Mbps: CPU (x264)".
  - NVIDIA constant-QP: `next_nvenc_qp` searches QP 16..42 over up to 4 attempts for the largest file that fits (accepts 93 to 100 % of the ceiling). Attempts go to temp files; the best one is copied to the output. If none fits, it falls back to NVENC bitrate mode, then to x264 two-pass (up to 4 passes, bitrate scaled each time).
  - Intel, AMD, VideoToolbox (bitrate-controlled): one corrective re-encode at 97 % of the ceiling, then x264 two-pass.
  - If nothing fits: the output file is deleted and the error is "Couldn't fit this video under X MB. Choose a larger size."
  - macOS: VideoToolbox (`h264_videotoolbox`, average bitrate) is tried first at any size; the 1 Mbps rule is exempt there. The Mac encoder list is disabled in Burn mode (Burn always uses VideoToolbox HEVC).
- 30-second test now works in Compress mode (`clip_start`, `clip_duration` on `CompressionInput`); the size ceiling is scaled to the clip so it uses the full encode's bitrate.
- `compress_video` is the whole job, independent of the Tauri window, so it can be tested from a terminal.
- Chips whose encoder was not detected are disabled. On Windows the Apple chip is hidden; on macOS NVIDIA/Intel/AMD are hidden.

## 5. Tried and dropped

- Left icon rail for Burn/Compress (the preview is horizontal; a sidebar wasted space). The switch moved under the preview, then into the dock.
- Layout concepts "guided steps", "editor dock with timeline", "drop first". Only the dock idea survived, rebuilt around a 16:9 preview.
- Preset-style and stepped (snapping) sliders. Replaced by continuous sliders with landmarks.
- Native range-input thumb styling (it did not render the capsule in the user's viewer). The thumb and track are drawn elements over an invisible native input.
- Per-slider hint paragraphs, the "Output" notes, the +/- target buttons (kept hidden for accessibility), the old Discord wording on the size heading.
- A fixed "aim 2 MB under" margin. Replaced by decimal MB plus 1 % from 5 MB up (covers services that count 1 MB as 1,000,000 bytes without wasting small budgets).
- A single NVIDIA correction pass (ended at about half the target). Replaced by the QP search.

## 6. Measured results

Machine: i7-12700K (20 threads), RTX 3060 Ti, Intel UHD 770, 31.8 GB RAM, ffmpeg 9.0.2 (gyan full build). Windows only; nothing was run on macOS or on AMD hardware.

Sources: the short checks use a generated `testsrc2` clip. The larger tests used the user's own NVIDIA ShadowPlay captures (Rocket League, plus Minecraft for the 10-minute file), 1080p60 at about 16 Mbps. Those captures are not in the repo. The first 20 % runs used a synthetic 1080p60 test pattern, not real footage.

Short synthetic checks (40 s, 1080p30): 8 MB target gave 7.6 MB; 4 MB gave 3.95 MB (ceiling 4,194,304); 1 MB was refused ("needs at least 2 MB"); 250 MB gave 46 MB from a 30.8 MB source (bitrate cap working); a 10 s clip of the 8 MB plan gave 1.9 MB. Forcing each encoder (CPU, Intel, NVIDIA, AMD which is not present and fell back to x264) passed all of these.

NVIDIA size search, 20 % of source, synthetic sources: before the fix both clips landed at 10.4 % (QP 32, half the budget); after the fix 17.8 %.

Real footage, 20 % target:

| Clip | Source | Output | Plan | SSIM / PSNR vs source | Time |
|---|---|---|---|---|---|
| 5 min | 592.5 MB | 116.1 MB (19.6 %) | 900p60 | 0.879 / 29.2 dB | 246 s |
| 10 min | 1180.4 MB | 213.5 MB (18.1 %) | 900p60 | 0.872 / 29.3 dB | 348 s |

Frame-by-frame on the 5-minute clip (900p60): median 29.6 dB, 61 % of frames under 30 dB, 1.9 % under 25 dB. This is a hard target: about 3 Mbps for fast gameplay.

900p60 vs 720p60 at about the same size (5-minute clip):

| Plan | Size | SSIM | PSNR mean | Frames under 30 dB |
|---|---|---|---|---|
| 900p60 | 116.1 MiB | 0.879 | 29.7 dB | 61.1 % |
| 720p60 (new rule) | 107.5 MiB | 0.886 | 29.8 dB | 58.3 % |
| 720p60, one QP lower | 120.7 MiB | 0.893 | 30.1 dB | 47.0 % |

The gain is small (about 0.4 dB at +4 % size); this is one clip of one game.

20 MB target, real footage (ceiling 19.8 million bytes):

| Clip | Output | Plan | Encoder | Time |
|---|---|---|---|---|
| 5 min | 18.9 MB (18,853,495 bytes) | 540p30, 0.43 Mbps | x264 two-pass | 213 s |
| 10 min | 19.3 MB (19,348,808 bytes) | 480p30, 0.18 Mbps | x264 two-pass | 812 s |

- NVIDIA could not hold 0.44 and 0.19 Mbps (it worked at 1.5 and 3 Mbps; 0.5 to 1.5 Mbps was not tested). That is why the 1 Mbps rule exists.
- Intel Quick Sync forced: fit 20 MB on the 5-minute clip (19.6 MB, 185 s) but failed on the 10-minute clip at 0.19 Mbps until the x264 fallback was added.
- The failed hardware attempts were not what made the 10-minute run slow: skipping them with the 1 Mbps rule took 777 s versus 771 s before. The x264 encode itself is the slow part.
- x264 preset test (2 minutes of the real footage at 190 kbps): quality `slow` 0.700 SSIM, `medium` 0.692, `fast` 0.677. Timings were inconsistent between runs (medium took 18 s in one and 80 s in the other), so no speed conclusion. The preset is still chosen by core count in `cpu_compression_tuning` (more than 8 cores means `slow`).

## 7. Not verified, known gaps, ideas

- macOS: nothing was built or run. `h264_videotoolbox` compression, its size accuracy at low bitrates, the Mac layout and the 792 px window height are untested. CI (`release.yml`) runs `npm test` on a macOS runner before building; the unit tests are written to pass there.
- AMD (AMF): not testable on this machine, so its low-bitrate behavior is unknown. The corrective re-encode and x264 fallback cover it.
- The UI was only checked in headless Edge screenshots with a stubbed backend, plus brief looks at the running app. Spot-check the live window.
- Not implemented: if the source file is already under the picked size, say so and skip re-encoding (today it re-encodes with the bitrate cap).
- Not implemented: judging picture quality (a short sample encode scored with SSIM/VMAF could warn on poor quality). The 150 kbps floor is a rough guess.
- The progress bar's behavior across several passes was not observed.
- The CPU path is slow for very small sizes on long videos (about 13 minutes for a 10-minute 1080p60 clip at 20 MB).
- Decided to leave the Burn preset dropdown removed. If it is brought back, rename it (no "CTT"), make clear it sets encoder effort and not quality, and note that Compress mode never used it.

## 8. File map

- `renderer/index.html`, `renderer/app.css`, `renderer/renderer.js`, `renderer/tauri-api.js`: the whole UI. Slider logic is `enhanceSlider` in `renderer.js`; estimate logic is `updateEstimate`; platform handling is `checkTools` and `refreshEncoderAvailability`.
- `src-tauri/src/media.rs`: all backend logic. Key items: `find_sidecar` / `find_subtitle`, `ceiling_bytes`, `check_compression_target`, `compression_plan_bytes`, `compression_geometry`, `plan_compression`, `use_cpu_encoder`, `automatic_compression_encoder`, `compression_hardware_recipe_with_qp`, `nvenc_vbr_args`, `next_nvenc_qp`, `compress_video`, `start_compression`.
- `src-tauri/src/lib.rs`: command registration, window sizing at startup.
- `src-tauri/tauri.conf.json`: fixed window (880x764, not resizable or maximizable).
- `STYLE_NOTES.md`: subtitle rendering research (unchanged). `README.md` and `changelogs.md` were updated.
