# A file larger than 4 GiB below D:\odir-test\bigdata (crates/onesimus-rig/tests/windows_fd.rs, big file test).
# Random blocks sit around the 32 bit limit (4 GiB) and at the end, zeros elsewhere (compresses well).
$ErrorActionPreference = 'Stop'
$d = 'D:\odir-test\bigdata'
if (Test-Path -LiteralPath $d) { cmd /c "rmdir /s /q `"$d`"" | Out-Null }
New-Item -ItemType Directory -Force $d | Out-Null
$size = 4GB + 600MB + 12345
$f = [IO.File]::Open("$d\huge.bin", [IO.FileMode]::Create, [IO.FileAccess]::ReadWrite)
try {
  $f.SetLength($size)
  $rnd = New-Object Random 7
  foreach ($off in @(0, (4GB - 65536), 4GB, (4GB + 12345), ($size - 1MB))) {
    $b = New-Object byte[] 1048576
    if ($off -eq 4GB - 65536) { $b = New-Object byte[] 262144 }
    $rnd.NextBytes($b)
    $f.Position = $off
    $f.Write($b, 0, [Math]::Min($b.Length, $size - $off))
  }
} finally { $f.Close() }
"huge.bin size " + (Get-Item "$d\huge.bin").Length
