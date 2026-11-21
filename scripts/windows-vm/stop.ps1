Get-Process ofd -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq 'D:\odir-test\bin\ofd.exe' } | Stop-Process -Force
Start-Sleep 1
"ofd running: " + [bool](Get-Process ofd -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq 'D:\odir-test\bin\ofd.exe' })
