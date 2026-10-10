# DMG 外观设置（macOS 出包用，由 dmgbuild 读取）。
#
# 为什么不用 Tauri 自带的 DMG：
#   Tauri 在 CI 上会给 create-dmg 传 `--skip-jenkins`，跳过"设置窗口背景与图标位置"的
#   AppleScript（headless runner 上 Finder 自动化不可靠，create-dmg 的作者也是这么建议的），
#   于是打出来的 DMG 就是「两个图标堆在窗口左上角、没有留白与箭头」的朴素样子
#   （用户反馈原话："安装的页面不对啊"）。
#   而 dmgbuild **自己写 .DS_Store**、不经过 Finder，所以能在这个 CI 上跑出正常的拖拽布局。
#
# 用法（工作目录必须是仓库根；CI 里由 release.yml 调用）：
#   python3 -m pip install --user dmgbuild
#   python3 -m dmgbuild -s scripts/dmg-settings.py "DBmind" out.dmg
import os

# 放进磁盘镜像的 .app（相对仓库根的路径）
app = 'target/release/bundle/macos/DBmind.app'
app_name = os.path.basename(app)

files = [app]
# 右侧那个「应用程序」软链 —— 有它才是"拖进去安装"，不是让用户自己想办法
symlinks = {'Applications': '/Applications'}

# 左：App；右：应用程序。横向拉开距离，中间就是"拖过去"的动线。
icon_locations = {
    app_name: (180, 190),
    'Applications': (480, 190),
}

# 压缩成只读镜像。**必须显式写**：dmgbuild 的默认格式是 UDRW（可读写、不压缩），
# 不写这一行出来的镜像会比 Tauri 那个大好几倍。
format = 'UDZO'
compression_level = 9

# 窗口位置与尺寸（宽度与 tauri.conf.json 的 bundle.macOS.dmg.windowSize 保持一致，
# 这样两套打包方式出来的观感是同一个）
window_rect = ((200, 120), (660, 400))
default_view = 'icon-view'
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
arrange_by = None
label_pos = 'bottom'
icon_size = 96

# 卷图标用桌面壳那份（就是 v1.2.1 起按 Apple 网格留白重做过的那张）
_icon = 'crates/dbmind-desktop/icons/icon.icns'
if os.path.exists(_icon):
    icon = _icon
