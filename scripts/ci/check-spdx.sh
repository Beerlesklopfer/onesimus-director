#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Prüft, dass jede Quell- und Skriptdatei den SPDX-Kopf AGPL-3.0-or-later trägt.
set -eu
cd "$(dirname "$0")/../.."
exec python3 scripts/ci/spdx.py check
