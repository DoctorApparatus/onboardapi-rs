#!/usr/bin/env bash
# Downloads the onboardapi C++ SDK release and unpacks one build into .sdk/<version>/<arch>-linux.
#
# usage: scripts/fetch-sdk.sh [version] [arch] [build-type]
#   version     default 9.10.0
#   arch        x86_64 | armv8 | armv7   (default: from uname -m)
#   build-type  Release | RelWithDebInfo | Debug (default Release)
set -euo pipefail

version="${1:-9.10.0}"
arch="${2:-$(uname -m)}"
build="${3:-Release}"

case "$arch" in
    x86_64) inner="onboardapi_${version}_cyclone_Linux_ubuntu_gcc9_libstdc++11_x86_64_${build}_shared.tar.gz" ;;
    aarch64 | armv8) arch=armv8; inner="onboardapi_${version}_cyclone_Linux_gcc7_libstdc++11_armv8_${build}_shared.tar.gz" ;;
    armv7*) arch=armv7; inner="onboardapi_${version}_cyclone_Linux_gcc7_libstdc++11_armv7_${build}_shared.tar.gz" ;;
    *) echo "unsupported arch: $arch" >&2; exit 1 ;;
esac

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dest="$root/.sdk/$version/$arch-linux"
if [[ -d "$dest/include/onboardapi" ]]; then
    echo "SDK already present: $dest"
    exit 0
fi

url="https://github.com/Rheinmetall/onboardapi/releases/download/v${version}/onboardapi_${version}_cpp-linux.tar.gz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "downloading $url (~400 MB)"
curl -fL --progress-bar -o "$tmp/sdk.tar.gz" "$url"
tar -xzf "$tmp/sdk.tar.gz" -C "$tmp" "$inner"
mkdir -p "$dest"
tar -xzf "$tmp/$inner" -C "$dest"
echo "SDK installed: $dest"
