#!/usr/bin/env bash
set -euo pipefail
# Private, reproducible audio-only FFmpeg. Does not replace system libraries.
rustune_repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
rustune_build_root=${RUSTUNE_BUILD_CACHE:-${XDG_CACHE_HOME:-"$HOME/.cache"}/rustune-build}
rustune_version=9.0.2
rustune_prefix="$rustune_build_root/ffmpeg-$rustune_version/install"
mkdir -p -- "$rustune_build_root"
python3 - "$rustune_build_root" "$rustune_version" <<'PY'
from pathlib import Path
import hashlib,sys,tarfile,urllib.request
root=Path(sys.argv[1]);version=sys.argv[2];archive=root/f'ffmpeg-{version}.tar.xz'
expected='8c3850283eb25fa026482078a04051e0be17347b09ef81a0849bec15a96e002e'
if not archive.exists():
    with urllib.request.urlopen(f'https://ffmpeg.org/releases/ffmpeg-{version}.tar.xz',timeout=30) as response,archive.open('wb') as output:
        total=0
        while chunk:=response.read(262144):
            total+=len(chunk)
            if total>25*1024*1024:raise RuntimeError('FFmpeg archive exceeds size bound')
            output.write(chunk)
if hashlib.sha256(archive.read_bytes()).hexdigest()!=expected:raise RuntimeError('FFmpeg source checksum mismatch')
if not (root/f'ffmpeg-{version}/configure').exists():
    with tarfile.open(archive) as source:source.extractall(root,filter='data')
PY
if [[ ! -f "$rustune_prefix/lib/pkgconfig/libavcodec.pc" ]]; then
    (
        cd -- "$rustune_build_root/ffmpeg-$rustune_version"
        ./configure --prefix="$rustune_prefix" --disable-everything --disable-autodetect \
            --disable-programs --disable-doc --disable-debug --enable-small --enable-static \
            --disable-shared --enable-pic --disable-x86asm --disable-avdevice --disable-avfilter \
            --disable-swscale --enable-avformat --enable-avcodec --enable-avutil --enable-swresample \
            --enable-decoder=aac --enable-parser=aac --enable-demuxer=mov \
            --enable-protocol=https,tls,tcp,http --enable-openssl --enable-pthreads
        make -j4
        make install
    )
fi
cd -- "$rustune_repo"
PKG_CONFIG_PATH="$rustune_prefix/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}" \
    "${CARGO:-$HOME/.cargo/bin/cargo}" build --locked --release --workspace --features rustune-audio/slim-ffmpeg
