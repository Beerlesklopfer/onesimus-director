# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Starts hold.ps1 detached and waits until the file is locked
Remove-Item D:\odir-test\lock.ready -ErrorAction SilentlyContinue
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = 'powershell -NoProfile -ExecutionPolicy Bypass -File D:\odir-test\hold.ps1'; CurrentDirectory = 'D:\odir-test' }
for ($i = 0; $i -lt 30 -and -not (Test-Path D:\odir-test\lock.ready); $i++) { Start-Sleep 1 }
"locked: " + (Test-Path D:\odir-test\lock.ready)
