#!/bin/sh
# Debian package ofd-plugin-postgresql (POSTGRESQL-PHASE2.md, step G).
#
# Input is an already signed bundle; this script never sees a private key.
# Signing happens before (docs/plans/SIGNIER-INFRASTRUKTUR.md).
#
#   packaging/ofd-plugin-postgresql/build-deb.sh <postgresql.pb> <onesimus-pg-broker> [outdir]
#
# Layout (ofd's default Plugin Directory is /usr/lib/<triplet>/onesimus/plugins):
#   /usr/lib/<triplet>/onesimus/plugins/postgresql.pb
#   /usr/lib/<triplet>/onesimus/plugins/postgresql/onesimus-pg-broker
# Every file and directory belongs to root and is not writable for group or
# others; ofd refuses the plugin otherwise (StrictModes up to "/").
# The postinst configures peer access of every PostgreSQL cluster for the
# broker and adds "Plugin Names = postgresql" to ofd, on every upgrade too.
set -eu

usage="usage: build-deb.sh <postgresql.pb> <onesimus-pg-broker> [outdir]"
bundle=${1:?$usage}
broker=${2:?$usage}
outdir=${3:-target/debian}

here=$(cd "$(dirname "$0")" && pwd)
top=$(cd "$here/../.." && pwd)

# Same version as ofd: a plugin is signed for exactly one suite version, and
# ofd refuses bundles of another one (SIGNIER-INFRASTRUKTUR.md, hard cut).
version=$(sed -n 's/^version = "\(.*\)"/\1/p' "$top/Cargo.toml" | head -n1)
[ -n "$version" ] || { echo "no workspace version in $top/Cargo.toml" >&2; exit 1; }
debversion="$version-1"
arch=$(dpkg --print-architecture)
triplet=$(dpkg-architecture -qDEB_HOST_MULTIARCH)

[ -s "$bundle" ] || { echo "bundle $bundle missing or empty" >&2; exit 1; }
[ -x "$broker" ] || { echo "broker $broker missing or not executable" >&2; exit 1; }
[ "$(od -An -tx1 -N4 "$broker" | tr -d ' ')" = 7f454c46 ] ||
    { echo "broker $broker is not an ELF executable" >&2; exit 1; }

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
plugins="$stage/usr/lib/$triplet/onesimus/plugins"
doc="$stage/usr/share/doc/ofd-plugin-postgresql"
install -d -m 0755 "$plugins/postgresql" "$doc" "$stage/DEBIAN"
install -m 0644 "$bundle" "$plugins/postgresql.pb"
install -m 0755 "$broker" "$plugins/postgresql/onesimus-pg-broker"
install -m 0644 "$top/packaging/copyright" "$doc/copyright"
install -m 0644 "$top/plugins/postgresql/README.md" "$doc/README.md"
install -m 0755 "$here/postinst" "$stage/DEBIAN/postinst"
install -m 0755 "$here/postrm" "$stage/DEBIAN/postrm"

size=$(du -sk --exclude=DEBIAN "$stage" | cut -f1)
cat > "$stage/DEBIAN/control" <<EOF
Package: ofd-plugin-postgresql
Version: $debversion
Architecture: $arch
Multi-Arch: same
Maintainer: Joerg Bernau <Joerg.Bernau@onesimus.io>
Installed-Size: $size
Depends: ofd (= $debversion), libc6, iproute2
Enhances: postgresql
Section: admin
Priority: optional
Homepage: https://onesimus.io
Description: signed PostgreSQL plugin for the Onesimus File Daemon
 Logical backups of local PostgreSQL instances (pg_dump -Fc per database,
 pg_dumpall --globals-only) without passwords: the broker runs as the
 instance's own user and connects over the local socket. The plugin is
 signed; ofd checks signature, suite version and the hash of the broker on
 every job. On install and upgrade the package lets every cluster owner log
 in with peer authentication and adds "Plugin Names = postgresql" to ofd.
 FileSet line: Plugin = "postgresql:instance=main".
EOF

mkdir -p "$outdir"
deb="$outdir/ofd-plugin-postgresql_${debversion}_${arch}.deb"
dpkg-deb --root-owner-group --build "$stage" "$deb" >/dev/null
echo "$deb"
