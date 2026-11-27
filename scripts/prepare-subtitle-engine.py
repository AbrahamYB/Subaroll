#!/usr/bin/env python3
"""Stage the pinned CPU-only Parakeet runtime for each supported desktop target."""
import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
DEST = ROOT / 'src-tauri' / 'binaries'
COMMIT = '01abde80dd018570f7590bf59fcb46a495e8c0c2'



def run(args, **kwargs):
    subprocess.run(args, check=True, **kwargs)


def main():
    DEST.mkdir(parents=True, exist_ok=True)
    if sys.platform == 'darwin':
        if os.uname().machine != 'arm64': raise SystemExit('Apple silicon is required.')
        out = DEST / 'subaroll-asr-aarch64-apple-darwin'
        stamp = DEST / 'subaroll-asr-macos.stamp'
        if out.is_file() and stamp.is_file():
            info = stamp.read_text().strip().split()
            if len(info) == 2 and info[0] == COMMIT and hashlib.sha256(out.read_bytes()).hexdigest() == info[1] and (DEST/'PARAKEET-LICENSE.txt').is_file():
                print('Using the previously built subtitle engine.')
                return
        cmake = shutil.which('cmake')
        if not cmake and Path('/tmp/subaroll-cmake/cmake/data/bin/cmake').is_file(): cmake = '/tmp/subaroll-cmake/cmake/data/bin/cmake'
        if not cmake: raise SystemExit('Install CMake to build the subtitle engine.')
        with tempfile.TemporaryDirectory(prefix='subaroll-engine-build-') as tmp:
            source, build = Path(tmp) / 'source', Path(tmp) / 'build'
            run(['git','clone','--no-checkout','https://github.com/mudler/parakeet.cpp.git',str(source)])
            run(['git','-C',str(source),'checkout','--detach',COMMIT])
            run(['git','-C',str(source),'submodule','update','--init','--depth','1','third_party/ggml'])
            env = dict(os.environ, MACOSX_DEPLOYMENT_TARGET='11.0')
            run([cmake,'-S',str(source),'-B',str(build),'-DCMAKE_BUILD_TYPE=Release','-DCMAKE_OSX_DEPLOYMENT_TARGET=11.0','-DBUILD_SHARED_LIBS=OFF','-DPARAKEET_WITH_CED=OFF','-DPARAKEET_WITH_VOICEDETECT=OFF','-DPARAKEET_BUILD_SERVER=OFF','-DPARAKEET_GGML_METAL=OFF','-DGGML_METAL=OFF','-DGGML_BLAS=OFF'],env=env)
            run([cmake,'--build',str(build),'--target','parakeet-cli','-j','4'],env=env)
            shutil.copy2(build/'examples'/'cli'/'parakeet-cli',out)
            shutil.copy2(source/'LICENSE',DEST/'PARAKEET-LICENSE.txt')
            stamp.write_text(COMMIT + ' ' + hashlib.sha256(out.read_bytes()).hexdigest() + '\n')
    elif sys.platform == 'win32':
        cmake = shutil.which('cmake')
        if not cmake: raise SystemExit('Install CMake and Visual Studio C++ tools to build the subtitle engine.')
        with tempfile.TemporaryDirectory(prefix='subaroll-engine-build-') as tmp:
            source, build = Path(tmp) / 'source', Path(tmp) / 'build'
            run(['git','clone','--no-checkout','https://github.com/mudler/parakeet.cpp.git',str(source)])
            run(['git','-C',str(source),'checkout','--detach',COMMIT])
            run(['git','-C',str(source),'submodule','update','--init','--depth','1','third_party/ggml'])
            run([cmake,'-S',str(source),'-B',str(build),'-A','x64','-DBUILD_SHARED_LIBS=OFF','-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded','-DPARAKEET_WITH_CED=OFF','-DPARAKEET_WITH_VOICEDETECT=OFF','-DPARAKEET_BUILD_SERVER=OFF','-DGGML_OPENMP=OFF'])
            run([cmake,'--build',str(build),'--config','Release','--target','parakeet-cli','-j','4'])
            shutil.copy2(build/'examples'/'cli'/'Release'/'parakeet-cli.exe',DEST/'subaroll-asr-x86_64-pc-windows-msvc.exe')
            shutil.copy2(source/'LICENSE',DEST/'PARAKEET-LICENSE.txt')
    else: raise SystemExit('Unsupported build platform')
    print('Prepared the Parakeet subtitle engine.')

if __name__ == '__main__': main()
