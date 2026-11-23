# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# cargo test of ofd on the Windows host, started detached by test.ps1; the filter is in D:\odir-test\test-filter.txt
$ErrorActionPreference = 'Continue'
Set-Location D:\odir-test\src
$env:PROTOC = 'D:\onesimus-ci\protoc\bin\protoc.exe'
$env:OPENSSL_DIR = 'D:\onesimus-ci\openssl'
$env:OPENSSL_NO_VENDOR = '1'
$env:CARGO_TARGET_DIR = 'D:\odir-test\target'
$env:PATH = "$env:USERPROFILE\.cargo\bin;D:\onesimus-ci\openssl\bin;" + $env:PATH
$env:ONESIMUS_WIN_TRUST = 'D:\odir-test\trust'
& "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Launch-VsDevShell.ps1" -VsInstallationPath "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools" -Arch amd64 -SkipAutomaticLocation | Out-Null
Remove-Item D:\odir-test\test.done -ErrorAction SilentlyContinue
$filter = (Get-Content D:\odir-test\test-filter.txt -Raw).Trim()
cargo test -p ofd $filter -- --nocapture --test-threads=1 2>&1 | Tee-Object -FilePath D:\odir-test\test.log
"exit=$LASTEXITCODE" | Out-File D:\odir-test\test.done
