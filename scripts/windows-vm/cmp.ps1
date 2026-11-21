$a='D:\odir-test\testdata'; $b='D:\odir-test\restore\D\odir-test\testdata'
"restore tree exists: " + (Test-Path $b)
$fa = Get-ChildItem $a -Recurse -File | Sort-Object FullName
$fb = Get-ChildItem $b -Recurse -File -ErrorAction SilentlyContinue | Sort-Object FullName
"files: source $($fa.Count), restored $($fb.Count)"
$bad = 0
foreach ($f in $fa) {
  $rel = $f.FullName.Substring($a.Length)
  $g = Join-Path $b $rel
  if (-not (Test-Path -LiteralPath $g)) { "MISSING $rel"; $bad++; continue }
  $h1 = (Get-FileHash -LiteralPath $f.FullName -Algorithm SHA256).Hash
  $h2 = (Get-FileHash -LiteralPath $g -Algorithm SHA256).Hash
  $t1 = $f.LastWriteTimeUtc.ToString('o'); $t2 = (Get-Item -LiteralPath $g).LastWriteTimeUtc.ToString('o')
  $same = ($h1 -eq $h2); $tsec = ($f.LastWriteTimeUtc.Ticks - (Get-Item -LiteralPath $g).LastWriteTimeUtc.Ticks)
  "{0,-18} hash {1}  size {2}/{3}  mtime delta ticks {4}" -f $rel.TrimStart('\'), $(if($same){'SAME'}else{'DIFFER'}), $f.Length, (Get-Item -LiteralPath $g).Length, $tsec
  if (-not $same) { $bad++ }
}
"dir mtime source/restored (testdata): " + (Get-Item $a).LastWriteTimeUtc.ToString('s') + " / " + (Get-Item $b).LastWriteTimeUtc.ToString('s')
"bad=$bad"
