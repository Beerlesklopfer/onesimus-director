# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
Get-Process ofd -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq 'D:\odir-test\bin\ofd.exe' } | Stop-Process -Force
Start-Sleep 1
Remove-Item -Recurse -Force D:\odir-test\src -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force D:\odir-test\src | Out-Null
tar -xzf D:\odir-test\odir-src.tgz -C D:\odir-test\src
Remove-Item D:\odir-test\build.done -ErrorAction SilentlyContinue
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = 'powershell -NoProfile -ExecutionPolicy Bypass -File D:\odir-test\build-inner.ps1'; CurrentDirectory = 'D:\odir-test' }
"rebuild started pid=$($r.ProcessId) rc=$($r.ReturnValue)"
