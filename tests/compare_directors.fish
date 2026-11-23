#!/usr/bin/env fish
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Wrapper for compare_directors.py
python3 (dirname (status filename))/compare_directors.py $argv
