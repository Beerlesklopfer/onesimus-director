# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Holds D:\odir-test\testdata\locked.bin open with no sharing for 120 s (started detached by lock.ps1)
$f = [IO.File]::Open('D:\odir-test\testdata\locked.bin', 'Create', 'ReadWrite', 'None')
$bytes = [Text.Encoding]::ASCII.GetBytes('locked content'); $f.Write($bytes, 0, $bytes.Length); $f.Flush()
Set-Content D:\odir-test\lock.ready 'ready'
Start-Sleep 120
$f.Close()
