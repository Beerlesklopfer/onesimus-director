Remove-Item -Recurse -Force D:\odir-ring-test\src -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force D:\odir-ring-test\src | Out-Null
tar -xzf D:\odir-ring-test\src.tgz -C D:\odir-ring-test\src
Remove-Item D:\odir-ring-test\test.done -ErrorAction SilentlyContinue
$r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = 'powershell -NoProfile -ExecutionPolicy Bypass -File D:\odir-ring-test\ofd-inner.ps1'; CurrentDirectory = 'D:\odir-ring-test' }
"started pid=$($r.ProcessId) rc=$($r.ReturnValue)"
