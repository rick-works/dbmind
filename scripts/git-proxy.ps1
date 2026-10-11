# 取「系统代理」并转成 git 的 -c 参数（没开代理就返回空数组 = 直连）。
#
# 为什么需要它：
#   Windows 上的代理开关在**注册表**里（ProxyEnable / ProxyServer），而 git 默认不读那里 ——
#   于是用户明明开着代理，git 直连 github.com:443 会一直超时（本项目多次踩到）。
#   反过来，脚本里写死 `-c http.proxy=`（把代理**关掉**）更糟：开代理的机器上永远连不上。
#   所以统一走这里：有代理就用，没有就直连。
#
# 用法（PowerShell 里把数组 splat 进原生命令）：
#   . "$PSScriptRoot\git-proxy.ps1"
#   $proxy = Get-GitProxyArgs
#   git -C $dir @proxy push $url main
function Get-GitProxyArgs {
    # 1) 环境变量（CI、或手动 export 过的 shell）
    foreach ($n in @('HTTPS_PROXY', 'https_proxy', 'HTTP_PROXY', 'http_proxy')) {
        $v = [Environment]::GetEnvironmentVariable($n)
        if ($v) { return @('-c', "http.proxy=$v", '-c', "https.proxy=$v") }
    }

    # 2) git 自己的配置
    $g = git config --get http.proxy 2>$null
    if ($g) { return @('-c', "http.proxy=$g", '-c', "https.proxy=$g") }

    # 3) Windows 系统代理（注册表）
    #    ProxyServer 可能是 "host:port"，也可能是 "http=host:port;https=host:port" 这种按协议分的写法
    if ($env:OS -eq 'Windows_NT') {
        $ie = Get-ItemProperty 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Internet Settings' -ErrorAction SilentlyContinue
        if ($ie -and $ie.ProxyEnable -eq 1 -and $ie.ProxyServer) {
            $s = [string]$ie.ProxyServer
            if ($s -match '=') {
                $s = ($s -split ';' |
                    Where-Object { $_ -match '^\s*https?=' } |
                    ForEach-Object { ($_ -split '=', 2)[1] } |
                    Select-Object -First 1)
            }
            if ($s) {
                # 没写协议就补 http:// （git 需要带协议）
                if ($s -notmatch '^[a-zA-Z]+://') { $s = "http://$s" }
                return @('-c', "http.proxy=$s", '-c', "https.proxy=$s")
            }
        }
    }

    # 没配代理：不给 -c，让 git 按自己的配置走（默认就是直连）
    return @()
}
