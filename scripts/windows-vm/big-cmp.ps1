# Compares D:\odir-test\bigdata\huge.bin with its restore (length and SHA-256)
$ErrorActionPreference = 'Continue'
$a = 'D:\odir-test\bigdata\huge.bin'; $b = 'D:\odir-test\restore\D\odir-test\bigdata\huge.bin'
"source length   " + (Get-Item $a).Length
if (-not (Test-Path -LiteralPath $b)) { "restore missing"; "bad=1"; exit }
"restored length " + (Get-Item $b).Length
$ha = (Get-FileHash $a -Algorithm SHA256).Hash; $hb = (Get-FileHash $b -Algorithm SHA256).Hash
"hash " + $(if ($ha -eq $hb) { 'SAME' } else { 'DIFFER' })
"bad=" + $(if ($ha -eq $hb -and (Get-Item $a).Length -eq (Get-Item $b).Length) { 0 } else { 1 })
