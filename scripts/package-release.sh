#!/usr/bin/env bash
set -euo pipefail

if [[ $# != 2 ]]; then
    echo "Usage: $0 BINARY_DIRECTORY OUTPUT_DIRECTORY" >&2
    exit 1
fi

repo=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
version=$(sed -nE 's/^version = "([0-9]+\.[0-9]+\.[0-9]+)"$/\1/p' "$repo/Cargo.toml")
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]
asset="rustune-v$version-x86_64-unknown-linux-gnu.tar.gz"
bin=$(cd -- "$1" && pwd)
mkdir -p -- "$2"
output=$(cd -- "$2" && pwd)
stage=$(mktemp -d -- "$output/.rustune-package.XXXXXXXX")
trap 'rm -r -- "$stage"' EXIT
export LC_ALL=C
unset TAR_OPTIONS GZIP

for program in rustune rustune-audio; do
    executable="$bin/$program"
    if [[ -L $executable || ! -f $executable || $("$executable" --version) != "$program $version" ]]; then
        echo "Expected a regular release binary for $program $version." >&2
        exit 1
    fi
    header=$(readelf -hW -- "$executable")
    if [[ $header != *ELF64* || $header != *'Advanced Micro Devices X86-64'* ]]; then
        echo "Expected an x86_64 ELF binary for $program." >&2
        exit 1
    fi
    segments=$(readelf -lW -- "$executable")
    if [[ $segments != *'/lib64/ld-linux-x86-64.so.2'* ]]; then
        echo "Expected the x86_64 glibc dynamic loader for $program." >&2
        exit 1
    fi
    dynamic=$(readelf -dW -- "$executable")
    if [[ $dynamic == *'libavcodec.so'* || $dynamic == *'libavformat.so'* ||
          $dynamic == *'libavutil.so'* || $dynamic == *'libswresample.so'* ]]; then
        echo "Build both release binaries with scripts/build-slim-audio.sh first." >&2
        exit 1
    fi
done

(
    cd -- "$stage"
    tar --format=ustar --mtime=@0 --owner=0 --group=0 --numeric-owner --mode=0755 \
        -C "$bin" -cf - rustune rustune-audio | gzip -n -9 > "$asset"
    sha256sum -- "$asset" > "$asset.sha256"
)
for file in "$asset" "$asset.sha256"; do
    mv -T -- "$stage/$file" "$output/$file"
done
