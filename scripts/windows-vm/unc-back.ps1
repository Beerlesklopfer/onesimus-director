# The restore onto the share (where=//localhost/odirunc/back) against the source of unc-prep.ps1 (Windows host)
$src = Get-ChildItem -LiteralPath D:\odir-test\uncdata -Recurse -File -Force | Where-Object { $_.FullName -notlike 'D:\odir-test\uncdata\back\*' } | ForEach-Object { (Get-FileHash -LiteralPath $_.FullName).Hash } | Sort-Object
$dst = Get-ChildItem -LiteralPath D:\odir-test\uncdata\back -Recurse -File -Force -ErrorAction SilentlyContinue | ForEach-Object { (Get-FileHash -LiteralPath $_.FullName).Hash } | Sort-Object
"back: $(@($dst).Count) files, same hashes: $(($src -join ',') -eq ($dst -join ','))"
