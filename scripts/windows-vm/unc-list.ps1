# Lists the files below D:\odir-test\uncrestore with their size and SHA-256, and the source of unc-prep.ps1
foreach ($root in 'D:\odir-test\uncdata', 'D:\odir-test\uncrestore') {
  "== $root"
  Get-ChildItem -LiteralPath $root -Recurse -File -Force | ForEach-Object {
    $rel = $_.FullName.Substring($root.Length)
    "{0} {1} {2}" -f $rel, $_.Length, (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash
  }
}
