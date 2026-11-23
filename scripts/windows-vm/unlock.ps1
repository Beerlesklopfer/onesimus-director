# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Ends the holder of lock.ps1 and removes the locked file
Get-CimInstance Win32_Process -Filter "Name = 'powershell.exe'" | Where-Object { $_.CommandLine -like '*D:\odir-test\hold.ps1*' } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
Start-Sleep 1
Remove-Item D:\odir-test\testdata\locked.bin -Force -ErrorAction SilentlyContinue
"unlocked"
