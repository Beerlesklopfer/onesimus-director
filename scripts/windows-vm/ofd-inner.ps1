# cargo test -p ofd win_trust -- --nocapture --test-threads=1 auf der Windows-VM, eigenes Verzeichnis D:\odir-ring-test (teilt nichts mit build-inner.ps1)
$ErrorActionPreference = 'Continue'
Set-Location D:\odir-ring-test\src
$env:PROTOC = 'D:\onesimus-ci\protoc\bin\protoc.exe'
$env:OPENSSL_DIR = 'D:\onesimus-ci\openssl'
$env:OPENSSL_NO_VENDOR = '1'
$env:CARGO_TARGET_DIR = 'D:\odir-ring-test\target'
$env:ONESIMUS_WIN_TRUST = 'D:\odir-test\trust'
$env:PATH = "$env:USERPROFILE\.cargo\bin;D:\onesimus-ci\openssl\bin;" + $env:PATH
& "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Launch-VsDevShell.ps1" -VsInstallationPath "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools" -Arch amd64 -SkipAutomaticLocation | Out-Null
cargo test -p ofd win_trust -- --nocapture --test-threads=1 2>&1 | Tee-Object -FilePath D:\odir-ring-test\test.log
"exit=$LASTEXITCODE" | Out-File D:\odir-ring-test\test.done
