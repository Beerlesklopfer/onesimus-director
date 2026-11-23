# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Changes the test tree after a full backup (crates/onesimus-rig/tests/windows_fd.rs, incremental test): D:\odir-test\testdata only
$ErrorActionPreference = 'Stop'
Start-Sleep 2
$t = 'D:\odir-test\testdata'
[IO.File]::WriteAllText("$t\a.txt", 'changed after the full backup')
[IO.File]::WriteAllText("$t\sub\new.txt", 'a new file')
"changed"
