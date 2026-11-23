#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Prüft, dass jede Quell- und Skriptdatei den SPDX-Kopf AGPL-3.0-or-later trägt.
set -eu
cd "$(dirname "$0")/../.."
# im CI-Container gehört der Checkout einem anderen Benutzer: git verweigert sonst `ls-files`
git config --global --add safe.directory "$(pwd)" 2>/dev/null || true
exec python3 scripts/ci/spdx.py check
