$dir = 'd:\develop\workspace\product\DBMind\_uitest'
$keep = @('user-edit-repro.cjs', 'start_dev.ps1', 'restart.ps1', 'cleanup.ps1', 'server.out.log', 'server.err.log')
Get-ChildItem $dir -File | Where-Object { $keep -notcontains $_.Name } | ForEach-Object {
  Remove-Item $_.FullName -Force -ErrorAction SilentlyContinue
}
'cleaned'
