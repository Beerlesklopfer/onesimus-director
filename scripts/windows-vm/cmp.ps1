# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
# Compares D:\odir-test\testdata with its restore below D:\odir-test\restore\D\odir-test\testdata (Windows host)
$ErrorActionPreference = 'Continue'
$a = 'D:\odir-test\testdata'; $b = 'D:\odir-test\restore\D\odir-test\testdata'
$sha = [Security.Cryptography.SHA256]::Create()
function V($p) { if ($p.Length -gt 240) { '\\?\' + $p } else { $p } }
function Hash($p) { $s = [IO.File]::OpenRead((V $p)); try { [BitConverter]::ToString($sha.ComputeHash($s)) } finally { $s.Close() } }
function Walk($root, $rel) {
  $dir = if ($rel) { "$root\$rel" } else { $root }
  foreach ($e in [IO.Directory]::GetFileSystemEntries((V $dir))) {
    $name = [IO.Path]::GetFileName($e)
    $r = if ($rel) { "$rel\$name" } else { $name }
    $attr = [IO.File]::GetAttributes($e)
    $r
    if (($attr -band [IO.FileAttributes]::Directory) -and -not ($attr -band [IO.FileAttributes]::ReparsePoint)) { Walk $root $r }
  }
}
"restore tree exists: " + (Test-Path -LiteralPath $b)
$sa = @(Walk $a '' | Sort-Object); $sb = @(Walk $b '' | Sort-Object)
"entries: source $($sa.Count), restored $($sb.Count)"
$bad = 0
foreach ($m in (Compare-Object $sa $sb)) { "ONLY " + $(if ($m.SideIndicator -eq '<=') { 'in source' } else { 'in restore' }) + ": " + $m.InputObject; $bad++ }
$files = 0
foreach ($r in $sa) {
  $pa = "$a\$r"; $pb = "$b\$r"
  if (-not ($sb -contains $r)) { continue }
  $aa = [IO.File]::GetAttributes((V $pa)); $ab = [IO.File]::GetAttributes((V $pb))
  $isLink = [bool]($aa -band [IO.FileAttributes]::ReparsePoint)
  if ($isLink) {
    $ia = Get-Item -LiteralPath $pa -Force; $ib = Get-Item -LiteralPath $pb -Force
    $same = ($ia.LinkType -eq $ib.LinkType) -and ($ib.Target -ne $null)
    "{0,-22} link {1}  source {2} -> {3}  restored {4} -> {5}" -f $r, $(if ($same) { 'SAME TYPE' } else { 'DIFFER' }), $ia.LinkType, ($ia.Target -join ','), $ib.LinkType, ($ib.Target -join ',')
    if (-not $same) { $bad++ }
    continue
  }
  if ($aa -band [IO.FileAttributes]::Directory) { continue }
  $files++
  $h = (Hash $pa) -eq (Hash $pb)
  $mask = [IO.FileAttributes]::ReadOnly -bor [IO.FileAttributes]::Hidden -bor [IO.FileAttributes]::System
  $attrSame = (($aa -band $mask) -eq ($ab -band $mask))
  $dm = ([IO.File]::GetLastWriteTimeUtc((V $pa)).Ticks - [IO.File]::GetLastWriteTimeUtc((V $pb)).Ticks)
  $dc = ([IO.File]::GetCreationTimeUtc((V $pa)).Ticks - [IO.File]::GetCreationTimeUtc((V $pb)).Ticks)
  $show = if ($r.Length -gt 22) { '...' + $r.Substring($r.Length - 19) } else { $r }
  "{0,-22} hash {1}  attrs {2}  mtime delta {3}  ctime delta {4}" -f $show, $(if ($h) { 'SAME' } else { 'DIFFER' }), $(if ($attrSame) { 'SAME' } else { "DIFFER ($aa / $ab)" }), $dm, $dc
  if (-not ($h -and $attrSame)) { $bad++ }
}
$n = { param($p) $o = fsutil hardlink list $p 2>&1; @($o | Where-Object { $_ -match '^\\' }).Count }
"hard links of a.txt: source " + (& $n "$a\a.txt") + ", restored " + (& $n "$b\a.txt")
if ((& $n "$a\a.txt") -ne (& $n "$b\a.txt")) { "hard link count DIFFERS"; $bad++ }
foreach ($r in 'acl.txt', 'emptyads.txt', 'a.txt') {
  $sa = (Get-Acl -LiteralPath "$a\$r").Sddl; $sb = (Get-Acl -LiteralPath "$b\$r").Sddl
  $na = @(Get-Item -LiteralPath "$a\$r" -Stream * | ForEach-Object { $_.Stream + '=' + $(if ($_.Stream -ne ':$DATA') { Get-Content -LiteralPath "$a\$r" -Stream $_.Stream } else { '' }) } | Sort-Object)
  $nb = @(Get-Item -LiteralPath "$b\$r" -Stream * | ForEach-Object { $_.Stream + '=' + $(if ($_.Stream -ne ':$DATA') { Get-Content -LiteralPath "$b\$r" -Stream $_.Stream } else { '' }) } | Sort-Object)
  "{0,-14} sddl {1}  streams {2} ({3})" -f $r, $(if ($sa -eq $sb) { 'SAME' } else { "DIFFER`n  $sa`n  $sb" }), $(if (($na -join '|') -eq ($nb -join '|')) { 'SAME' } else { 'DIFFER' }), ($na -join ',')
  if ($r -eq 'acl.txt' -and $sa -notmatch '\(D;;[^)]*;;;BG\)') { "source ACL of acl.txt was not set (test setup)"; $bad++ }
  if ($sa -ne $sb -or ($na -join '|') -ne ($nb -join '|')) { $bad++ }
}
$da = (Get-Acl -LiteralPath "$a\aclsub").Sddl; $db = (Get-Acl -LiteralPath "$b\aclsub").Sddl
"directory aclsub: source has its own ACL: " + ($da -match '\(A;OICI;[^)]*;;;BU\)') + "; sddl " + $(if ($da -eq $db) { 'SAME' } else { 'DIFFER' })
if ($da -ne $db) { $bad++ }
"files compared: $files"
"bad=$bad"
