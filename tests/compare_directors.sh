#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Wrapper for compare_directors.py
exec python3 "$(dirname "$0")/compare_directors.py" "$@"
