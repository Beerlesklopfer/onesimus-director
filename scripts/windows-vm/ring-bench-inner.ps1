# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# cargo test -p osd-ring --release bench auf der Windows-VM (Durchsatz und Rundlaufzeit des Rings)
$ErrorActionPreference = 'Continue'
Set-Location D:\odir-ring-test\src
$env:PROTOC = 'D:\onesimus-ci\protoc\bin\protoc.exe'
$env:OPENSSL_DIR = 'D:\onesimus-ci\openssl'
$env:OPENSSL_NO_VENDOR = '1'
$env:CARGO_TARGET_DIR = 'D:\odir-ring-test\target'
$env:PATH = "$env:USERPROFILE\.cargo\bin;" + $env:PATH
& "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Launch-VsDevShell.ps1" -VsInstallationPath "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools" -Arch amd64 -SkipAutomaticLocation | Out-Null
cargo test -p osd-ring --release -- --ignored --nocapture bench 2>&1 | Tee-Object -FilePath D:\odir-ring-test\test.log
"exit=$LASTEXITCODE" | Out-File D:\odir-ring-test\test.done
