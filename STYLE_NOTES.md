# Rendering research

The format name is **ASS** (Advanced SubStation Alpha), not AAS.

## Why IINA is the reference

IINA forwards its subtitle preferences to mpv. Its source maps the visible controls to mpv's `sub-font`, `sub-font-size`, `sub-outline-size`, `sub-scale`, `sub-delay`, and `sub-pos` options. mpv renders ASS through libass.

IINA's current built-in plain-text defaults are:

- Font: `sans-serif`
- Size: `55`
- Text: white
- Border: black, width `3`
- Shadow: `0`
- Horizontal alignment: centered
- Vertical alignment: bottom
- Horizontal margin: `25`
- Vertical margin: `22`
- Position: `100` (the authored/original position)

The app uses those values for plain subtitle formats, with Arial as the concrete cross-platform `sans-serif` face. The script resolution is normalized to 1280×720 for 16:9 video, matching mpv's definition of size in pixels scaled to a 720-line frame.

## Why ASS files are preserved

Crunchyroll playback responses expose subtitle tracks as `.ass` files. ASS carries its own style table and can also carry per-line positioning and drawing commands. There is no single safe universal “Crunchyroll font” to stamp over every file; overriding an authored ASS can break signs, songs, placement, and show-specific typesetting.

For `.ass` input, this app therefore keeps every authored color, outline, shadow, alignment, font name, and positioned sign. At the defaults (100% size, Original position, 0 delay), it sends the authored ASS to libass unchanged. Size and position only adjust normal dialogue styles; lines with explicit `\\pos`, `\\move`, or top/middle alignment commands are treated as signs and left alone.

Font files (`.ttf`, `.otf`, `.ttc`, `.otc`) placed beside the subtitle file are loaded automatically. Exact rendering still depends on having the fonts named by the ASS file.

## Timing

The app does not estimate subtitle timing from frames. It uses the subtitle event timestamps and preserves source video timing (`fps_mode=passthrough`). The delay control changes only the timestamp seen by libass and then restores the original video timestamps; audio is not shifted. Preview frames use the same prepared ASS file and renderer as the final export.

## Primary references

- IINA preference defaults: https://github.com/iina/iina/blob/develop/iina/Preference.swift
- IINA-to-mpv option mapping: https://github.com/iina/iina/blob/develop/iina/MPVController.swift
- mpv subtitle option behavior: https://mpv.io/manual/stable/#subtitles
- FFmpeg libass subtitle filter: https://ffmpeg.org/ffmpeg-filters.html#subtitles-1
- Example Crunchyroll playback subtitle model (`.ass` URLs): https://github.com/CuteTenshii/crunchyroll-downloader/blob/master/episode.go
