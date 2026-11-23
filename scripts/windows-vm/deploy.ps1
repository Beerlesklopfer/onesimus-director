# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
Get-Process ofd -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq 'D:\odir-test\bin\ofd.exe' } | Stop-Process -Force
Start-Sleep 1
Copy-Item D:\odir-test\target\debug\ofd.exe D:\odir-test\bin\ofd.exe -Force
(Get-Item D:\odir-test\bin\ofd.exe).LastWriteTime
