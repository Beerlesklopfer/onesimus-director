#!/bin/sh
# Debian package ofd-plugin-openldap (docs/plans/OPENLDAP-PLUGIN.md).
#
# Input is an already signed bundle; this script never sees a private key.
# Signing happens before (docs/plans/SIGNIER-INFRASTRUKTUR.md): plugin id
# io.onesimus.openldap, capabilities file.read, file.write, openldap.read,
# openldap.write, native part openldap/onesimus-ldap-broker.
#
#   packaging/ofd-plugin-openldap/build-deb.sh <openldap.pb> <onesimus-ldap-broker> [outdir]
#
# Layout (ofd's default Plugin Directory is /usr/lib/<triplet>/onesimus/plugins):
#   /usr/lib/<triplet>/onesimus/plugins/openldap.pb
#   /usr/lib/<triplet>/onesimus/plugins/openldap/onesimus-ldap-broker
# Every file and directory belongs to root and is not writable for group or
# others; ofd refuses the plugin otherwise (StrictModes up to "/").
# The postinst adds "Plugin Names = openldap" to ofd, on every upgrade too.
# It never touches slapd: the broker runs slapcat as the slapd user and
# needs no ACL, group or password.
set -eu

usage="usage: build-deb.sh <openldap.pb> <onesimus-ldap-broker> [outdir]"
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
chmod 0755 "$stage"
trap 'rm -rf "$stage"' EXIT
plugins="$stage/usr/lib/$triplet/onesimus/plugins"
doc="$stage/usr/share/doc/ofd-plugin-openldap"
install -d -m 0755 "$plugins/openldap" "$doc" "$stage/DEBIAN"
install -m 0644 "$bundle" "$plugins/openldap.pb"
install -m 0755 "$broker" "$plugins/openldap/onesimus-ldap-broker"
install -m 0644 "$top/packaging/copyright" "$doc/copyright"
install -m 0644 "$top/plugins/openldap/README.md" "$doc/README.md"
install -m 0755 "$here/postinst" "$stage/DEBIAN/postinst"
install -m 0755 "$here/postrm" "$stage/DEBIAN/postrm"

size=$(du -sk --exclude=DEBIAN "$stage" | cut -f1)
cat > "$stage/DEBIAN/control" <<EOF
Package: ofd-plugin-openldap
Version: $debversion
Architecture: $arch
Multi-Arch: same
Maintainer: Joerg Bernau <Joerg.Bernau@onesimus.io>
Installed-Size: $size
Depends: ofd (= $debversion), libc6, iproute2
Recommends: slapd
Enhances: slapd
Section: admin
Priority: optional
Homepage: https://onesimus.io
Description: signed OpenLDAP plugin for the Onesimus File Daemon
 Consistent backups of a local slapd (slapd.d or slapd.conf): the
 configuration and one slapcat LDIF per mdb database, without passwords.
 The broker runs slapcat as the user slapd runs as, never as root. Restore
 goes to files, or online with slapadd into a new, empty target that ofd
 creates; the running server is never written. The plugin is signed; ofd
 checks signature, suite version and the hash of the broker on every job.
 On install and upgrade the package adds "Plugin Names = openldap" to ofd;
 slapd is not changed. FileSet line: Plugin = "openldap:instance=main".
EOF

mkdir -p "$outdir"
deb="$outdir/ofd-plugin-openldap_${debversion}_${arch}.deb"
dpkg-deb --root-owner-group --build "$stage" "$deb" >/dev/null
echo "$deb"
