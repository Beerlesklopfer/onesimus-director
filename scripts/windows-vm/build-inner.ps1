$ErrorActionPreference = 'Continue'
Set-Location D:\odir-test\src
$env:PROTOC = 'D:\onesimus-ci\protoc\bin\protoc.exe'
$env:OPENSSL_DIR = 'D:\onesimus-ci\openssl'
$env:OPENSSL_NO_VENDOR = '1'
$env:CARGO_TARGET_DIR = 'D:\odir-test\target'
$env:PATH = "$env:USERPROFILE\.cargo\bin;" + $env:PATH
& "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\Launch-VsDevShell.ps1" -VsInstallationPath "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools" -Arch amd64 -SkipAutomaticLocation | Out-Null
Remove-Item D:\odir-test\build.done -ErrorAction SilentlyContinue
cargo build -p ofd 2>&1 | Tee-Object -FilePath D:\odir-test\build.log
"exit=$LASTEXITCODE" | Out-File D:\odir-test\build.done
