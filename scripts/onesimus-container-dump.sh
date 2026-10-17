#!/bin/bash
# onesimus-container-dump.sh — Container-Dump vor der Sicherung (Podman und Docker)
#
# Gedacht als ClientRunBeforeJob des Bareos/Onesimus-Jobs. Schreibt nach
# DUMP_DIR (Standard /var/backups/containers), das im FileSet enthalten sein muss:
#
#   <engine>/inventory/   Images (mit Digests), Container, Volumes, Netzwerke und
#                          die vollständige inspect-Ausgabe der Container (JSON) —
#                          damit lässt sich die Umgebung nach einem Restore neu
#                          aufbauen (Images aus der Registry neu ziehen).
#   <engine>/images/       je Image-ID ein Archiv (mit allen Tags) der lokal gebauten Images (ohne RepoDigest einer echten
#                          Registry, also nie gezogen oder gepusht; Podman-Digests
#                          "localhost/…" zählen als lokal) als
#                          gzip-komprimiertes `save`-Archiv. Ein Archiv wird nur
#                          neu geschrieben, wenn sich die Image-ID geändert hat,
#                          damit inkrementelle Sicherungen klein bleiben; Archive
#                          entfernter Images werden gelöscht.
#
# Engines: docker (falls vorhanden), podman als root (rootful) und optional
# rootless Podman je Benutzer aus PODMAN_USERS (z. B. PODMAN_USERS="jbernau").
#
# Aufruf:  onesimus-container-dump.sh [DUMP_DIR]
# Exit:    0 = alles gesichert (fehlende Engines sind kein Fehler),
#          1 = mindestens ein Image oder Inventar konnte nicht geschrieben werden.

set -u
DUMP_DIR=${1:-${DUMP_DIR:-/var/backups/containers}}
PODMAN_USERS=${PODMAN_USERS:-}
umask 077
failures=0

log() { echo "container-dump: $*"; }
fail() { echo "container-dump: FEHLER: $*" >&2; failures=$((failures + 1)); }

# run_engine <label> <command…>: führt die Engine als passender Benutzer aus.
dump_engine() {
    local label=$1; shift
    local -a engine=("$@")
    local dir="$DUMP_DIR/$label"
    mkdir -p "$dir/inventory" "$dir/images" || { fail "$dir nicht anlegbar"; return; }

    # Inventar: immer vollständig neu (klein).
    "${engine[@]}" image ls --digests --no-trunc > "$dir/inventory/images.txt" 2>&1 \
        || fail "$label: image ls"
    "${engine[@]}" ps -a --no-trunc > "$dir/inventory/containers.txt" 2>&1 \
        || fail "$label: ps"
    "${engine[@]}" volume ls > "$dir/inventory/volumes.txt" 2>&1 || true
    "${engine[@]}" network ls > "$dir/inventory/networks.txt" 2>&1 || true
    local ids
    ids=$("${engine[@]}" ps -aq 2>/dev/null)
    if [ -n "$ids" ]; then
        # shellcheck disable=SC2086
        "${engine[@]}" container inspect $ids > "$dir/inventory/containers-inspect.json" 2>&1 \
            || fail "$label: container inspect"
    else
        echo "[]" > "$dir/inventory/containers-inspect.json"
    fi

    # Lokal gebaute Images: getaggt und ohne RepoDigest einer echten Registry.
    # Ein Archiv je Image-ID mit allen Tags dieser ID.
    declare -A tags_of=()
    local ref id
    while IFS=' ' read -r ref id; do
        [ -n "$ref" ] || continue
        case "$ref" in *"<none>"*|localhost/podman-pause:*) continue ;; esac
        # Lokal = kein RepoDigest einer echten Registry. Docker lässt RepoDigests
        # bei lokalen Builds leer, Podman trägt "localhost/<name>@sha256:…" ein.
        local digests remote=no d
        digests=$("${engine[@]}" image inspect --format '{{range .RepoDigests}}{{.}} {{end}}' "$ref" 2>/dev/null) \
            || { fail "$label: image inspect $ref"; continue; }
        for d in $digests; do
            case "$d" in localhost/*) ;; *) remote=yes ;; esac
        done
        [ "$remote" = no ] || continue
        tags_of[$id]="${tags_of[$id]:-} $ref"
    done < <("${engine[@]}" image ls --format '{{.Repository}}:{{.Tag}} {{.ID}}' 2>/dev/null)

    local keep=" " multi=()
    [ "${engine[-1]}" = podman ] && multi=(-m)
    for id in "${!tags_of[@]}"; do
        # shellcheck disable=SC2206
        local refs=(${tags_of[$id]})
        local name
        name=$(printf '%s' "${refs[0]}" | tr '/:@' '___')
        keep="$keep$name "
        local archive="$dir/images/$name.tar.gz" idfile="$dir/images/$name.id"
        if [ -f "$archive" ] && [ "$(cat "$idfile" 2>/dev/null)" = "$id" ]; then
            log "$label: ${refs[*]} unverändert ($id)"
            continue
        fi
        log "$label: sichere ${refs[*]} ($id)"
        if "${engine[@]}" image save "${multi[@]}" "${refs[@]}" | gzip -c > "$archive.tmp" \
            && [ "${PIPESTATUS[0]}" -eq 0 ]; then
            mv -f "$archive.tmp" "$archive" && printf '%s\n' "$id" > "$idfile"
        else
            rm -f "$archive.tmp"; fail "$label: image save ${refs[*]}"
        fi
    done

    # Archive nicht mehr vorhandener lokaler Images entfernen.
    local f base
    for f in "$dir"/images/*.tar.gz; do
        [ -e "$f" ] || continue
        base=$(basename "$f" .tar.gz)
        case "$keep" in *" $base "*) ;; *)
            log "$label: entferne veraltetes Archiv $base"
            rm -f "$f" "$dir/images/$base.id" ;;
        esac
    done
}

mkdir -p "$DUMP_DIR" || { echo "container-dump: $DUMP_DIR nicht anlegbar" >&2; exit 1; }

if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
    dump_engine docker docker
else
    log "docker nicht vorhanden oder nicht erreichbar — übersprungen"
fi

if command -v podman >/dev/null 2>&1; then
    dump_engine podman podman
    for user in $PODMAN_USERS; do
        if id "$user" >/dev/null 2>&1; then
            dump_engine "podman-$user" runuser -u "$user" -- podman
        else
            fail "Benutzer $user unbekannt"
        fi
    done
else
    log "podman nicht vorhanden — übersprungen"
fi

[ "$failures" -eq 0 ] || exit 1
exit 0
