$env:DBMIND_JAVA = 'D:\develop\tools\jdk-25.0.4.1+1\bin\java.exe'
$wd = 'd:\develop\workspace\product\DBMind'
Start-Process -FilePath "$wd\target\debug\dbmind-web.exe" `
  -ArgumentList '--port','20362','--dist',"$wd\frontend\dist" `
  -WorkingDirectory $wd `
  -WindowStyle Hidden `
  -RedirectStandardOutput "$wd\_uitest\server.out.log" `
  -RedirectStandardError "$wd\_uitest\server.err.log"
Start-Sleep -Seconds 3
$c = Get-NetTCPConnection -LocalPort 20362 -State Listen -ErrorAction SilentlyContinue
if ($c) { "dev backend listening, pid=$($c[0].OwningProcess)" } else { 'NOT listening'; Get-Content "$wd\_uitest\server.err.log" -Tail 5 }
