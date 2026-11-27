# Third-party notices

## DiscordCompressor credit

Subaroll's target-size compression feature is based on the workflow and
design of [vladaad's DiscordCompressor](https://github.com/vladaad/discordcompressor),
with full credit to vladaad for the original compression approach.
DiscordCompressor is licensed under the GNU Affero General Public License v3.0.
Subaroll does not bundle the DiscordCompressor executable or its Go
source code; its compression implementation is independently written in Rust.

## FFmpeg

The macOS release bundles a static Apple-silicon build of FFmpeg and FFprobe
from [aagedal/ffmpeg-apple-silicon v9.0.2-1](https://github.com/aagedal/ffmpeg-apple-silicon/releases/tag/v9.0.2-1),
archive `ffmpeg-9.0.2-full-macos-arm64.zip`, SHA-256
`004ed3719259c9ce973b82ae399ada187bba62b2eba0c09e7751524b15cbaec3`.
The corresponding GPL license accompanies the bundled tools.

The Windows release bundles `ffmpeg.exe` and `ffprobe.exe` from the pinned
[BtbN FFmpeg GPL build](https://github.com/BtbN/FFmpeg-Builds/releases/tag/autobuild-2026-11-14-12-37),
archive `ffmpeg-N-128909-g604ef3784d-win64-gpl.zip`, SHA-256
`1adf262a8238d059a3a6e1949f026ba4b071bfa29b2399af53557cbc63008658`.

FFmpeg is a separate program distributed under the GNU General Public License.
Its source code is available from [FFmpeg](https://ffmpeg.org/download.html) and
the exact build scripts are available from
[BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds). Subaroll
invokes FFmpeg as a subprocess and does not modify FFmpeg.

## Interface fonts

IBM Plex Sans and IBM Plex Mono are distributed under the SIL Open
Font License 1.1. The font files and license texts are bundled in
[renderer/fonts](renderer/fonts).

## Automatic subtitle generation

The optional model is [Parakeet Redux](https://huggingface.co/moondream/parakeet-redux),
derived by Moondream from NVIDIA Parakeet TDT v3. The model is distributed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). The bundled
[parakeet.cpp](https://github.com/mudler/parakeet.cpp) runtime is MIT licensed;
its license accompanies the application. The GGUF model is a converted copy of
the original model, not a new training run.
