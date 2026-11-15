# Subtitle Burner

A lightweight Rust/Tauri macOS and Windows tool for permanently burning text subtitles into video with FFmpeg and libass—the same subtitle-rendering family used by mpv/IINA.

## What it preserves

- `.ass`: keeps the subtitle file's fonts, colors, outlines, shadows, alignment, and signs. The Size and Position controls adjust dialogue while leaving positioned signs alone.
- `.ssa`: converts to ASS, preserving authored styling where FFmpeg supports it.
- `.srt`, `.vtt`, `.sub`, `.smi`: applies the fixed IINA-like plain-text preset: Arial, size 55 at a 720-line script resolution, white text, 3 px black outline, no shadow, bottom-center alignment.
- The preview frame and final export use the same prepared ASS and libass filter.
- Subtitle delay changes subtitle evaluation time without shifting the video or audio streams.

This project is not affiliated with Crunchyroll or IINA. Crunchyroll-delivered ASS files contain their own authored styling; keeping that styling is more exact than guessing a universal Crunchyroll preset.

## Run

1. Install a full FFmpeg build containing the `subtitles`/libass filter.
   - macOS with Homebrew: `brew install ffmpeg-full`
   - Windows: install a full FFmpeg build and add its `bin` folder to PATH, or choose `ffmpeg.exe` inside the app.
2. Install Rust with `rustup`, then run `npm install` once.
3. Run `npm start`.

The app automatically prefers Homebrew's `ffmpeg-full` on macOS. If FFmpeg is elsewhere, click the renderer badge and choose it.

## Install a release

- **macOS 11 or newer, Apple silicon only:** download the ARM64 `.dmg`, drag **Subtitle Burner** into Applications, then right-click the app and choose **Open** the first time. Install the media engine once with `brew install ffmpeg-full`.
- **Windows 11 x64:** download the `setup.exe` from GitHub Releases and run it. The installer includes the tested FFmpeg/FFprobe build, so no separate media-tool installation is required. Until the project has a paid code-signing certificate, Windows SmartScreen may require **More info → Run anyway**.

Release builds are created natively by GitHub Actions: ARM64 on macOS and x64 on Windows 11. No Electron runtime is included.

## Build folders

- macOS: `npm run build:mac`
- Windows (run on Windows): `npm run build:win`

The native bundles are written below `src-tauri/target/release/bundle/`. The public Windows installer bundles the pinned GPL FFmpeg build documented in `THIRD_PARTY_NOTICES.md`; local builds and the macOS package use an installed FFmpeg.

## Export behavior

- Subtitle burn-in preserves source resolution, aspect ratio, frame timing, HDR/10-bit signaling, metadata, and chapters. macOS uses HEVC VideoToolbox. Windows detects NVIDIA NVENC, AMD AMF, or Intel Quick Sync and falls back to x264/x265 when required.
- Target-size compression automatically chooses output bitrate and resolution. On Windows, Auto prefers a usable hardware AV1 encoder (NVIDIA NVENC, Intel Quick Sync, then AMD AMF), with an explicit H.264 compatibility option. NVIDIA uses CQP 20 and allows one calculated QP correction pass if the result exceeds the selected size. Unavailable hardware falls back through H.264 hardware and then adaptive CPU x264 two-pass encoding.
- GPU recipes are initialized at the intended output resolution before an export begins, so an encoder merely appearing in FFmpeg's list is not treated as usable.
- MP4 output uses AAC audio for compatibility. Metadata and chapters are retained where the output container supports them.
