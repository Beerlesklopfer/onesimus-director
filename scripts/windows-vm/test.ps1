# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# starts test-inner.ps1 detached (WMI), after unpacking odir-src.tgz and preparing the trust directories
Remove-Item -Recurse -Force D:\odir-test\src -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force D:\odir-test\src | Out-Null
tar -xzf D:\odir-test\odir-src.tgz -C D:\odir-test\src
Remove-Item D:\odir-test\test.done -ErrorAction SilentlyContinue
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = 'powershell -NoProfile -ExecutionPolicy Bypass -File D:\odir-test\test-inner.ps1'; CurrentDirectory = 'D:\odir-test' }
"test started pid=$($r.ProcessId) rc=$($r.ReturnValue)"
