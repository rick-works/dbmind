$c = Get-NetTCPConnection -LocalPort 20362 -State Listen -ErrorAction SilentlyContinue
if ($c) { Stop-Process -Id $c[0].OwningProcess -Force; 'killed old backend' }
Set-Location 'd:\develop\workspace\product\DBMind'
cargo build -p dbmind-web 2>&1 | Out-Null
$env:DBMIND_JAVA = 'D:\develop\tools\jdk-25.0.4.1+1\bin\java.exe'
$wd = 'd:\develop\workspace\product\DBMind'
Start-Process -FilePath "$wd\target\debug\dbmind-web.exe" `
  -ArgumentList '--port','20362','--dist',"$wd\frontend\dist" `
  -WorkingDirectory $wd -WindowStyle Hidden `
  -RedirectStandardOutput "$wd\_uitest\server.out.log" `
  -RedirectStandardError "$wd\_uitest\server.err.log"
Start-Sleep -Seconds 3
$c2 = Get-NetTCPConnection -LocalPort 20362 -State Listen -ErrorAction SilentlyContinue
if ($c2) { "dev backend listening pid=$($c2[0].OwningProcess)" } else { 'NOT LISTENING' }
