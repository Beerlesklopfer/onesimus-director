#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Baut osd als statisches 32-Bit-Programm (i686, musl) und ein Debian-Paket
# für i386, z. B. für ein altes NAS mit 32-Bit-Kernel (Plan osd, „32-Bit-Hosts“).
#
# Voraussetzungen (einmalig, ohne root):
#   rustup target add i686-unknown-linux-musl
#   python3 -m venv ~/.local/zigenv && ~/.local/zigenv/bin/pip install ziglang cargo-zigbuild
#   cargo install cargo-deb   (falls nicht vorhanden)
# Aufruf:  ZIGENV=~/.local/zigenv scripts/build-osd-i686.sh
#
# Warum so:
# - zig liefert C-Compiler und musl-Header für i686; ein Debian-Host hat
#   beides nicht ohne root (musl-dev:i386).
# - OpenSSL (TLS-PSK für BNET, rustls kann kein PSK) wird mit dem Feature
#   `vendored-openssl` aus den Quellen gebaut und statisch gelinkt.
# - `-DBROKEN_CLANG_ATOMICS`: OpenSSL ruft sonst `__atomic_is_lock_free`
#   auf, das die zig-Laufzeit für i686 nicht enthält; OpenSSL nimmt dann
#   seinen Mutex-Weg.
set -eu

target=i686-unknown-linux-musl
zigenv=${ZIGENV:-$HOME/.local/zigenv}
[ -x "$zigenv/bin/cargo-zigbuild" ] || { echo "cargo-zigbuild fehlt in $zigenv (siehe Kopf des Skripts)" >&2; exit 1; }
cd "$(dirname "$0")/.."

export PATH="$zigenv/bin:$PATH"
export CFLAGS_i686_unknown_linux_musl=-DBROKEN_CLANG_ATOMICS
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-4}

cargo zigbuild --release --target "$target" -p osd-daemon -p outils --features osd-daemon/vendored-openssl
file "target/$target/release/osd"

rev=$(git rev-parse --short HEAD)
cargo deb -p osd-daemon --target "$target" --no-build --no-strip \
    --output "target/debian/osd-${rev}-i386.deb"
dpkg-deb -I "target/debian/osd-${rev}-i386.deb" | grep -E 'Package|Version|Architecture'
