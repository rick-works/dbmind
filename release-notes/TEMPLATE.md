# 发版说明模板（只说更新了什么 + 安装包表格）

> 用法：跑完 `node scripts/release.mjs <版本> <级别> <日期>` 之后，照这个骨架写
> `release-notes/v<版本>.md`；表格不用手写，交给脚本：
>
> ```
> node scripts/fill-release-assets.mjs v<版本>     # 从 Release API 读真实资产，补出表格
> gh release edit v<版本> --repo rick-works/dbmind --title v<版本> --notes-file release-notes/v<版本>.md
> ```
>
> **仓库里这份文件就是 Release 页面上的正文**（原样搬上去）。所以正文里只放两样东西：
> ① 这一版改了什么；② 安装包表格（哪个文件适合谁、多大）。不放发布日期、类型标签、
> 分节的实现解释、Linux 安装说明、升级提示 —— 页面上左侧就是版本号、下面就是资产列表，
> 这些要么重复、要么读者不关心。

---

## 更新说明

> 这个标题由 `node scripts/fill-release-assets.mjs <版本>` 统一补上（和下面的表格一起），
> 手写时**只写要点**即可。

- 一句话说清第一件事
- 一句话说清第二件事
- 一句话说清第三件事

## 安装包

> 这一段由 `node scripts/fill-release-assets.mjs <版本>` 生成（顺序固定：Windows → macOS → Linux，
> 只列该版本实际发出来的文件，体积取自 Release API）。手别改，改了下次跑脚本会被覆盖。

| 平台 | 文件 | 形态与适用场景 | 大小 |
|---|---|---|---|
| Windows | DBmind_<版本>_x64-setup.exe | 安装向导（推荐） | XX MB |
| Windows | DBmind_<版本>_x64_en-US.msi | MSI 包（企业批量部署 / 静默安装） | XX MB |
| Windows | dbmind-<版本>-portable.zip | **免安装**：解压即用，不写注册表 | XX MB |
| macOS（Apple 芯片） | DBmind_<版本>_aarch64.dmg | 安装镜像：把图标拖进「应用程序」 | XX MB |
| macOS（Intel） | DBmind_<版本>_x64.dmg | 安装镜像：把图标拖进「应用程序」 | XX MB |
| Linux（Debian / Ubuntu） | DBmind_<版本>_amd64.deb | apt 安装，带开始菜单项 | XX MB |
| Linux（通用发行版） | DBmind_<版本>_amd64.AppImage | **免安装**：`chmod +x` 后直接运行 | XX MB |

- 每个包都自带精简 Java 运行时（目标机器不必另装 Java）。

---

## 要点这部分怎么写

1. **只列更新内容**：2~5 条，每条一行；不用小标题，不写日期、不写"修订版/新增功能"这类标签。
2. **一句话说完**：开场白一律不要（"本次更新"、"我们优化了…"）。
3. **用户视角**：说行为与结果，不说实现 —— 写"点关闭不再没反应"，别写"补了 ACL 权限声明"。
4. **短的优先**：要点部分一两屏读完最好。
5. 确实值得补充的细节（例如 Linux 依赖），写进 README，不塞进 Release。

## 渲染器支持什么（应用内的更新弹窗也用它）

标题、段落、粗体、斜体、行内代码、无序/有序列表、引用块、分割线、表格、代码块都正常。

**不支持**（写了会退化成纯文本）：图片、原始 HTML、任务列表、嵌套列表。

## 两条沿用约定

1. **重要的放最前面**：应用内的更新弹窗只显示开头一段，超出的会折叠成「查看完整更新说明」。
2. **校验值别写进正文**：SHA256 放成 Release 资产，正文里不放哈希。
