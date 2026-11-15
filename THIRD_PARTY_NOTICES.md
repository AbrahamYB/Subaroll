# Third-party notices

## DiscordCompressor credit

Subaroll's target-size compression feature is based on the workflow and
design of [vladaad's DiscordCompressor](https://github.com/vladaad/discordcompressor),
with full credit to vladaad for the original compression approach.
DiscordCompressor is licensed under the GNU Affero General Public License v3.0.
Subaroll does not bundle the DiscordCompressor executable or its Go
source code; its compression implementation is independently written in Rust.

## FFmpeg

The Windows release bundles `ffmpeg.exe` and `ffprobe.exe` from the pinned
[BtbN FFmpeg GPL build](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-11-14-12-37),
archive `ffmpeg-N-128909-g604ef3784d-win64-gpl.zip`, SHA-256
`1adf262a8238d059a3a6e1949f026ba4b071bfa29b2399af53557cbc63008658`.

FFmpeg is a separate program distributed under the GNU General Public License.
Its source code is available from [FFmpeg](https://ffmpeg.org/download.html) and
the exact build scripts are available from
[BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds). Subaroll
invokes FFmpeg as a subprocess and does not modify FFmpeg.
