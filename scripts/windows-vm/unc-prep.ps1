# Creates the SMB share odirunc (D:\odir-test\uncdata) on this host and fills it: the UNC path \\localhost\odirunc stands for a network drive
$ErrorActionPreference = 'Continue'
$elevated = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
"elevated: $elevated"
if (Test-Path D:\odir-test\uncdata) { Remove-Item -Recurse -Force D:\odir-test\uncdata }
New-Item -ItemType Directory -Force D:\odir-test\uncdata\sub | Out-Null
[IO.File]::WriteAllText('D:\odir-test\uncdata\a.txt', 'unc a')
[IO.File]::WriteAllText(('D:\odir-test\uncdata\sub\' + [char]0x00FC + 'n' + [char]0x00EF + '.txt'), 'unc unicode')
$b = New-Object byte[] 3000000; (New-Object Random 7).NextBytes($b); [IO.File]::WriteAllBytes('D:\odir-test\uncdata\sub\big.bin', $b)
if (-not (Get-SmbShare -Name odirunc -ErrorAction SilentlyContinue)) { New-SmbShare -Name odirunc -Path D:\odir-test\uncdata -FullAccess (New-Object Security.Principal.SecurityIdentifier 'S-1-1-0').Translate([Security.Principal.NTAccount]).Value | Out-Null }
Remove-Item -Recurse -Force D:\odir-test\uncrestore -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force D:\odir-test\uncrestore | Out-Null
"share: " + (Get-SmbShare -Name odirunc -ErrorAction SilentlyContinue).Path
"read: " + [IO.File]::ReadAllText('\\localhost\odirunc\a.txt')
