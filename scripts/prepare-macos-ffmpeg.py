#!/usr/bin/env python3
"""Download and verify portable Apple-silicon FFmpeg tools for app bundles."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parent.parent
DEST = ROOT / 'src-tauri' / 'binaries'
URL = 'https://github.com/aagedal/ffmpeg-apple-silicon/releases/download/v9.0.2-1/ffmpeg-9.0.2-full-macos-arm64.zip'
SHA256 = '004ed3719259c9ce973b82ae399ada187bba62b2eba0c09e7751524b15cbaec3'
ARCHIVE = Path(os.environ.get('RUNNER_TEMP', '/tmp')) / 'subaroll-ffmpeg-9.0.2-macos-arm64.zip'
DIRECTORY = 'ffmpeg-9.0.2-full-macos-arm64/'


def digest(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def main():
    if sys.platform != 'darwin' or os.uname().machine != 'arm64':
        raise SystemExit('Preparing this bundle requires an Apple-silicon Mac.')
    if not ARCHIVE.exists() or digest(ARCHIVE) != SHA256:
        print('Downloading pinned FFmpeg for Apple silicon...', flush=True)
        with tempfile.NamedTemporaryFile(dir=ARCHIVE.parent, delete=False) as output:
            pending = Path(output.name)
        try:
            subprocess.run(['curl', '--fail', '--location', '--retry', '3', '--output', str(pending), URL], check=True)
            if digest(pending) != SHA256:
                raise ValueError('FFmpeg archive checksum mismatch')
            pending.replace(ARCHIVE)
        finally:
            pending.unlink(missing_ok=True)
    DEST.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(ARCHIVE) as archive:
        for name in ('ffmpeg', 'ffprobe'):
            target = DEST / f'{name}-aarch64-apple-darwin'
            with archive.open(DIRECTORY + name) as source, target.open('wb') as output:
                shutil.copyfileobj(source, output)
            target.chmod(0o755)
        with archive.open(DIRECTORY + 'LICENSE') as source, (DEST / 'FFMPEG-LICENSE.txt').open('wb') as output:
            shutil.copyfileobj(source, output)
    ffmpeg = DEST / 'ffmpeg-aarch64-apple-darwin'
    ffprobe = DEST / 'ffprobe-aarch64-apple-darwin'
    def output(*args):
        return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT)
    config = output(str(ffmpeg), '-version')
    if '--enable-nonfree' in config:
        raise SystemExit('Refusing to package a non-redistributable FFmpeg build.')
    if ' subtitles ' not in output(str(ffmpeg), '-hide_banner', '-filters'):
        raise SystemExit('FFmpeg does not support libass subtitles.')
    encoders = output(str(ffmpeg), '-hide_banner', '-encoders')
    if 'h264_videotoolbox' not in encoders or 'libx264' not in encoders:
        raise SystemExit('FFmpeg is missing a required encoder.')
    output(str(ffprobe), '-version')
    for binary in (ffmpeg, ffprobe):
        deps = output('otool', '-L', str(binary))
        if '/opt/homebrew/' in deps or '/usr/local/' in deps:
            raise SystemExit(f'{binary.name} depends on a local installation.')
    print('Verified portable FFmpeg and FFprobe for macOS 11+ and Apple silicon.')

if __name__ == '__main__':
    main()
