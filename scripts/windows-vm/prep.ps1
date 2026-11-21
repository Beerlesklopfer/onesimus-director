Get-Process ofd -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq 'D:\odir-test\bin\ofd.exe' } | Stop-Process -Force
Start-Sleep 1
Remove-Item -Recurse -Force D:\odir-test\conf,D:\odir-test\work,D:\odir-test\testdata,D:\odir-test\restore -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force D:\odir-test\conf\director,D:\odir-test\conf\client,D:\odir-test\conf\messages,D:\odir-test\work,D:\odir-test\testdata\sub,D:\odir-test\restore | Out-Null
Set-Content -Encoding ascii D:\odir-test\conf\director\bareos-dir.conf 'Director { Name = bareos-dir; Password = "vfd-pw" }'
Set-Content -Encoding ascii D:\odir-test\conf\client\myself.conf 'FileDaemon { Name = winfd; FD Port = 19102; Working Directory = D:/odir-test/work; Pid Directory = D:/odir-test/work }'
Set-Content -Encoding ascii D:\odir-test\conf\messages\Standard.conf 'Messages { Name = Standard; Director = bareos-dir = all, !skipped, !restored }'
[IO.File]::WriteAllText('D:\odir-test\testdata\a.txt','hello windows')
[IO.File]::WriteAllBytes('D:\odir-test\testdata\empty.txt',[byte[]]@())
[IO.File]::WriteAllText([IO.Path]::Combine('D:\odir-test\testdata',"u" + [char]0x00FC + "mlaut-" + [char]0x00E9 + ".txt"),'unicode name')
$rnd = New-Object Random 42; $b = New-Object byte[] 3145728; $rnd.NextBytes($b); [IO.File]::WriteAllBytes('D:\odir-test\testdata\sub\b.bin',$b)
$b = New-Object byte[] 20971520; $rnd.NextBytes($b); [IO.File]::WriteAllBytes('D:\odir-test\testdata\big.bin',$b)
Get-ChildItem D:\odir-test\testdata -Recurse -File | Get-FileHash -Algorithm SHA256 | ForEach-Object { $_.Hash.Substring(0,16) + "  " + $_.Path.Substring(18) }
Remove-Item D:\odir-test\fd.out -ErrorAction SilentlyContinue
$cmd = 'cmd /c "D:\odir-test\bin\ofd.exe -f -c D:\odir-test\conf > D:\odir-test\fd.out 2>&1"'
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = $cmd; CurrentDirectory = 'D:\odir-test' }
Start-Sleep 3; Get-Content D:\odir-test\fd.out -TotalCount 3 | ForEach-Object { $_.Substring(0, [Math]::Min(150, $_.Length)) }
