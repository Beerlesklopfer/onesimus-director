#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Isolated Podman for the O-7 end-to-end test of the OpenLDAP plugin
# (docs/plans/OPENLDAP-PLUGIN.md, O-7): the packaged ofd runs as a real
# daemon in a slapd container (--network host), odir, bareos-sd and the
# catalog run on the host (crates/onesimus-rig). Driven by the ignored test
#
#   cargo test -p onesimus-rig --test ldap_o7 -- --ignored --nocapture
#
# which calls this script for every podman command:
#
#   o7-podman.sh init    <base>               create the isolated store
#   o7-podman.sh p       <base> <podman args> podman on that store
#   o7-podman.sh cleanup <base>               containers, images, store, pause process
#
# Isolation from the user's Podman (mandatory, same as run-in-podman.sh): an
# own --root, --runroot, --tmpdir, XDG_RUNTIME_DIR (0700), CONTAINERS_CONF
# with file locks and cgroupfs, all below <base>, which must be a directory
# /tmp/ldaptest-*. Never "system reset"; cleanup is rm -af, rmi -af,
# removing the own store and stopping the own pause process. Container names
# start with ldaptest-.
set -eu

usage="usage: o7-podman.sh init|p|cleanup <base> [podman args]"
cmd=${1:?$usage}
base=${2:?$usage}
shift 2
case $base in
/tmp/ldaptest-*/*) ;;
*) echo "o7-podman.sh: base $base is not below /tmp/ldaptest-*" >&2; exit 2 ;;
esac

P() {
    env XDG_RUNTIME_DIR="$base/xdg" TMPDIR="$base/tmp" CONTAINERS_CONF="$base/containers.conf" \
        podman --root "$base/root" --runroot "$base/run" --tmpdir "$base/tmp" --storage-driver overlay "$@"
}

case $cmd in
init)
    mkdir -p -m 700 "$base"
    chmod 700 "$base"
    mkdir -p -m 700 "$base/root" "$base/run" "$base/tmp" "$base/xdg"
    cat > "$base/containers.conf" <<EOF
[engine]
lock_type = "file"
cgroup_manager = "cgroupfs"
events_logger = "file"
EOF
    ;;
p)
    [ -f "$base/containers.conf" ] || { echo "o7-podman.sh: run init first" >&2; exit 2; }
    P "$@"
    ;;
cleanup)
    [ -f "$base/containers.conf" ] || exit 0
    P rm -af -t 0 >/dev/null 2>&1 || true
    P rmi -af >/dev/null 2>&1 || true
    P unshare rm -rf "$base/root" >/dev/null 2>&1 || true
    if [ -f "$base/xdg/libpod/tmp/pause.pid" ]; then
        kill "$(cat "$base/xdg/libpod/tmp/pause.pid")" 2>/dev/null || true
    fi
    rm -rf "$base" 2>/dev/null || true
    ;;
*)
    echo "$usage" >&2
    exit 2
    ;;
esac
