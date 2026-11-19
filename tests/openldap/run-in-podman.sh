#!/bin/sh
# End-to-end tests of the OpenLDAP plugin in two throwaway Podman containers
# (debian:trixie, slapd 2.6): ldaptest-olc (slapd.d) and ldaptest-conf
# (slapd.conf). Called by the ignored test ldap_e2e::podman_both_configurations
# of ofd; by hand:
#
#   tests/openldap/run-in-podman.sh <ofd test binary> <openldap.wasm> <onesimus-ldap-broker>
#
# Isolation from the user's Podman (mandatory): an own --root, --runroot,
# --tmpdir, XDG_RUNTIME_DIR (0700), CONTAINERS_CONF with file locks and
# cgroupfs, all below one mktemp directory under /tmp. Never "system reset";
# cleanup is rm -af, rmi -af, removing the own directory and stopping the
# own pause process. Container names start with ldaptest-.
set -eu

usage="usage: run-in-podman.sh <ofd test binary> <openldap.wasm> <onesimus-ldap-broker>"
test_bin=${1:?$usage}
wasm=${2:?$usage}
broker=${3:?$usage}
here=$(cd "$(dirname "$0")" && pwd)

base=$(mktemp -d /tmp/ldaptest-XXXXXXXX)
chmod 700 "$base"
mkdir -m 700 "$base/root" "$base/run" "$base/tmp" "$base/xdg"
cat > "$base/containers.conf" <<EOF
[engine]
lock_type = "file"
cgroup_manager = "cgroupfs"
events_logger = "file"
EOF

P() {
    env XDG_RUNTIME_DIR="$base/xdg" TMPDIR="$base/tmp" CONTAINERS_CONF="$base/containers.conf" \
        podman --root "$base/root" --runroot "$base/run" --tmpdir "$base/tmp" --storage-driver overlay "$@"
}

cleanup() {
    P rm -af -t 0 >/dev/null 2>&1 || true
    P rmi -af >/dev/null 2>&1 || true
    P unshare rm -rf "$base/root" >/dev/null 2>&1 || true
    if [ -f "$base/xdg/libpod/tmp/pause.pid" ]; then
        kill "$(cat "$base/xdg/libpod/tmp/pause.pid")" 2>/dev/null || true
    fi
    rm -rf "$base" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "== building localhost/ldaptest-slapd (isolated store $base)"
P build -q -t localhost/ldaptest-slapd "$here" >/dev/null

for kind in ${LDAPTEST_KINDS:-olc conf}; do
    c=ldaptest-$kind
    echo "== $c: setup"
    P run -d --init --name "$c" --hostname "$kind" localhost/ldaptest-slapd >/dev/null
    P exec "$c" "/opt/ldaptest/setup-$kind.sh"
    P exec "$c" mkdir -p /opt/t
    P cp "$test_bin" "$c:/opt/t/ofd-test"
    P cp "$wasm" "$c:/opt/t/openldap.wasm"
    P cp "$broker" "$c:/opt/t/onesimus-ldap-broker"
    echo "== $c: tests"
    P exec -e ONESIMUS_LDAP_E2E="$kind" -e ONESIMUS_LDAP_WASM=/opt/t/openldap.wasm \
        -e ONESIMUS_LDAP_BROKER=/opt/t/onesimus-ldap-broker -e RUST_BACKTRACE=1 \
        "$c" /opt/t/ofd-test --include-ignored --test-threads=1 --nocapture ldap_e2e::inside
done
echo "== all container tests passed"
