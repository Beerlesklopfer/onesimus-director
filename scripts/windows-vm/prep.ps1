# Test tree and ofd start for crates/onesimus-rig/tests/windows_fd.rs (runs on the Windows host, in D:\odir-test only)
$ErrorActionPreference = 'Stop'
Get-Process ofd -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq 'D:\odir-test\bin\ofd.exe' } | Stop-Process -Force
Start-Sleep 1
foreach ($d in 'D:\odir-test\conf','D:\odir-test\work','D:\odir-test\testdata','D:\odir-test\restore') {
  if (Test-Path -LiteralPath $d) { cmd /c "rmdir /s /q `"$d`"" | Out-Null }
}
New-Item -ItemType Directory -Force D:\odir-test\conf\director,D:\odir-test\conf\client,D:\odir-test\conf\messages,D:\odir-test\work,D:\odir-test\testdata\sub,D:\odir-test\restore | Out-Null
Set-Content -Encoding ascii D:\odir-test\conf\director\bareos-dir.conf 'Director { Name = bareos-dir; Password = "vfd-pw" }'
Set-Content -Encoding ascii D:\odir-test\conf\client\myself.conf 'FileDaemon { Name = winfd; FD Port = 19102; Working Directory = D:/odir-test/work; Pid Directory = D:/odir-test/work }'
Set-Content -Encoding ascii D:\odir-test\conf\messages\Standard.conf 'Messages { Name = Standard; Director = bareos-dir = all, !skipped, !restored }'
$t = 'D:\odir-test\testdata'
[IO.File]::WriteAllText("$t\a.txt", 'hello windows')
[IO.File]::WriteAllBytes("$t\empty.txt", [byte[]]@())
[IO.File]::WriteAllText([IO.Path]::Combine($t, "u" + [char]0x00FC + "mlaut-" + [char]0x00E9 + ".txt"), 'unicode name')
$rnd = New-Object Random 42
$b = New-Object byte[] 3145728; $rnd.NextBytes($b); [IO.File]::WriteAllBytes("$t\sub\b.bin", $b)
$b = New-Object byte[] 20971520; $rnd.NextBytes($b); [IO.File]::WriteAllBytes("$t\big.bin", $b)
# attributes
[IO.File]::WriteAllText("$t\ro.txt", 'read only'); [IO.File]::SetAttributes("$t\ro.txt", [IO.FileAttributes]::ReadOnly)
[IO.File]::WriteAllText("$t\hid.txt", 'hidden');  [IO.File]::SetAttributes("$t\hid.txt", [IO.FileAttributes]::Hidden -bor [IO.FileAttributes]::Archive)
# an access control list of its own and an alternate data stream; an empty file with a stream
[IO.File]::WriteAllText("$t\acl.txt", 'acl file')
# account names differ by language: SIDs only (the current user, Users, Guests)
$me = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
icacls "$t\acl.txt" /inheritance:r /grant:r "*${me}:(F)" /grant "*S-1-5-32-545:(R)" /deny "*S-1-5-32-546:(W)" | Out-Null
if ($LASTEXITCODE -ne 0) { throw "icacls failed on acl.txt" }
Set-Content -Path "$t\acl.txt" -Stream second -Value 'alternate data'
[IO.File]::WriteAllBytes("$t\emptyads.txt", [byte[]]@())
Set-Content -Path "$t\emptyads.txt" -Stream only -Value 'stream of an empty file'
# a directory with an access control list of its own (known gap: directories are not read with BackupRead)
New-Item -ItemType Directory -Path "$t\aclsub" | Out-Null
[IO.File]::WriteAllText("$t\aclsub\f.txt", 'in an acl dir')
icacls "$t\aclsub" /inheritance:r /grant:r "*${me}:(OI)(CI)(F)" /grant "*S-1-5-32-545:(OI)(CI)(R)" | Out-Null
if ($LASTEXITCODE -ne 0) { throw "icacls failed on aclsub" }
# times that are not "now": 2001-02-03 04:05:06.789 modified, 1999-12-31 23:59:58 created
[IO.File]::WriteAllText("$t\old.txt", 'old')
[IO.File]::SetLastWriteTimeUtc("$t\old.txt", [DateTime]::new(2001, 2, 3, 4, 5, 6, 789, [DateTimeKind]::Utc))
[IO.File]::SetCreationTimeUtc("$t\old.txt", [DateTime]::new(1999, 12, 31, 23, 59, 58, [DateTimeKind]::Utc))
# links
New-Item -ItemType HardLink -Path "$t\hard1.txt" -Target "$t\a.txt" | Out-Null
New-Item -ItemType SymbolicLink -Path "$t\link.txt" -Target "$t\a.txt" | Out-Null
New-Item -ItemType SymbolicLink -Path "$t\dlink" -Target "$t\sub" | Out-Null
New-Item -ItemType Junction -Path "$t\junc" -Target "$t\sub" | Out-Null
# a path longer than 260 characters
$deep = "$t\long"
for ($i = 0; $i -lt 8; $i++) { $deep += '\' + ('d' + $i + ('x' * 36)) }
[IO.Directory]::CreateDirectory('\\?\' + $deep) | Out-Null
[IO.File]::WriteAllText('\\?\' + $deep + '\deep.txt', 'deep file')
"deep path length: " + ($deep + '\deep.txt').Length
Remove-Item D:\odir-test\fd.out -ErrorAction SilentlyContinue
$cmd = 'cmd /c "D:\odir-test\bin\ofd.exe -f -c D:\odir-test\conf > D:\odir-test\fd.out 2>&1"'
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = $cmd; CurrentDirectory = 'D:\odir-test' }
Start-Sleep 3
Get-Content D:\odir-test\fd.out -TotalCount 3 | ForEach-Object { $_.Substring(0, [Math]::Min(150, $_.Length)) }
