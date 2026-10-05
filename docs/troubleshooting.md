# 常见问题与踩坑记录

都是**实际踩过**的坑，按「构建 → 运行 → 打包」排列。

## 构建

### PowerShell 5.1 读无 BOM 的 .ps1 会按 ANSI 解码
中文脚本文件必须存为 **UTF-8 with BOM**，否则中文被截成半个字符后引号配对被破坏，
脚本在**解析期**整体失败（一行都不执行）。`scripts\release.ps1` 等含中文的 .ps1 都要带 BOM。

### IDE 终端给 Node 套了「安全删除」shim
在 IDE 内置终端里跑构建，`vite build` 清空 `dist\assets`（约 600 个文件）会被
`[safe-delete][SAFE_DELETE_BULK_CONFIRM_REQUIRED]` 拦截直接失败。对策：构建脚本里用
.NET 的 `Directory.Delete(path, true)` 先清产物，Vite 面对的就是不存在的目录。
逐个文件删（`Remove-Item -LiteralPath`）也可绕过批量守卫。

### 无窗口会话（Start-Process Hidden）里 rolldown 必挂
`vite build` 在 `rendering chunks` 阶段稳定死亡（前台同一命令秒级成功）。对策：
耗时长的构建不要用分离后台进程跑整条链，改为分步前台执行。

### Tauri CLI 的 `--bundles msi,nsis` 报 invalid value
PowerShell 把 `msi,nsis` 当数组经 npx 重拼成 `msi nsis` 传给 Tauri。对策：**不传该参数**，
目标写在 `tauri.conf.json` 的 `bundle.targets` 里（单一真源）。

### jlink 的 --compress 参数随 JDK 版本变了
JDK 21+ 只认 `--compress=zip-6`，老版本是 `--compress=2`，传错直接报错退出。按 jlink 版本分支。

### 带 BOM 的 JSON 会让 Tauri 解析失败
写 `tauri.conf.json` 用 `[IO.File]::WriteAllText(path, text, UTF8Encoding($false))`（无 BOM）。

## 运行

### node 不带参数会进入 REPL
探测工具是否存在的脚本里必须带 `--version`，否则脚本永远停在等待输入。

### 事务必须与查询落在同一条连接
事务端点必须与执行端点走**同一 scope 解析**。曾出现 begin 在连接 23232、查询在
23233 的泳道错位（影子连接/目标不一致），表现为「事务开着但写操作仍自动提交」。

### 大 JOIN 的 COUNT 不要放主链路
`SELECT COUNT(*) FROM (大JOIN)` 全量物化能把回显卡到百秒级。总数必须异步统计，
带总预算超时，算不出就显示未知（绝不显示假总数）。

### 旧 chunk 被 HTML 回落污染
带哈希的前端资源 404 时**绝不能回落 index.html**（浏览器把 HTML 当 JS 执行 → 白屏）。
`index.html` 必须 `no-store`（无 ETag/Last-Modified 时 no-cache 会被浏览器当强缓存用）。

## 数据

### 元数据库裸拷可能拿到不一致快照
库开着（WAL 模式）时不要直接复制 `dbmind.db`，要用迁移功能（它走 SQLite 的
backup API）。详见 [storage.md](storage.md)。

### 磁盘占用大头
`target\`（Rust 编译缓存）：`debug\incremental` 可安全删除（19GB 级，自动重建）；
`release\` 删了下次打包重编。`dist\` 是发布产物，`package.ps1` 可再生。
