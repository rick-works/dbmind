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
# 首次打开会被 macOS 拦一次（未公证），把说明一起放进镜像 —— 用户被拦的那一刻，
# 这份说明就在同一个窗口里，不用再去网上翻步骤。
_first_launch = 'resources/mac/首次打开请看这里.txt'
if os.path.exists(_first_launch):
    files.append(_first_launch)
else:
    print('warn: 没找到首次打开说明（resources/mac/首次打开请看这里.txt），镜像里只有 App')

# 右侧那个「应用程序」软链 —— 有它才是"拖进去安装"，不是让用户自己想办法
symlinks = {'Applications': '/Applications'}

# 左：App；右：应用程序。横向拉开距离，中间就是"拖过去"的动线；
# 说明书放在下方居中（y 向下增大）。
# 图标 128pt，坐标点指的是**图标图片的中心**：图片占 y-64 … y+64，文件名标在图片下方
# （约 y+66 … y+96，长名字可能折成两行）。由此得两条约束：
#   · 上下两排之间要留够 —— 上一排的**文件名**不能被下一排的图标压住；
#   · 说明书那一排的 y 必须让"文件名"落在窗口**内**，否则就是"图标看得见、下面的字看不见"
#     （v1.2.7 的 dmg 正是如此：窗口只有 400 高、说明书放在 y=350 → 文件名整条都在窗外，
#      用户反馈"下面的字 看不见了"）。
# 现在按窗口 660×440 排：app 那排占 76…236，说明书那排占 246…406，整体在窗口里居中。
icon_locations = {
    app_name: (180, 140),
    'Applications': (480, 140),
    '首次打开请看这里.txt': (330, 310),
}

# 压缩成只读镜像。**必须显式写**：dmgbuild 的默认格式是 UDRW（可读写、不压缩），
# 不写这一行出来的镜像会比 Tauri 那个大好几倍。
format = 'UDZO'
compression_level = 9

# 窗口位置与尺寸（宽度与 tauri.conf.json 的 bundle.macOS.dmg.windowSize 保持一致，
# 这样两套打包方式出来的观感是同一个）
# 位置 (390, 230)：按 **1440×900 居中**算的 —— 那是 M1 的 Air / 13" Pro 的逻辑分辨率
#   （横向 (1440-660)/2 = 390，纵向 (900-440)/2 = 230）。注意 dmgbuild 的 y 是**从屏幕底部**算的，
#   不过"居中"这一点两种算法同值（屏幕对折对称），所以这个数不用纠结方向。
# 为什么不做成真正的"屏幕居中"：dmgbuild 文档明说做不到
#   （"it doesn't appear to be possible to position the window relative to the top left
#     or relative to the centre of the user's screen"），只能给一个固定点；
#   给偏了也不怕 —— Finder 会把窗口夹回可视区（文档原话），小屏上只会被挪正。
# 原先的 (200, 120) 是明显偏左下的，用户反馈应该居中。
window_rect = ((390, 230), (660, 440))
default_view = 'icon-view'
show_status_bar = False
show_tab_view = False
show_toolbar = False
show_pathbar = False
show_sidebar = False
arrange_by = None
label_pos = 'bottom'
# 128 = macOS 磁盘镜像里"两个大图标"的经典尺寸（用户口径：不要密密麻麻的小图标）
icon_size = 128

# 卷图标用桌面壳那份（就是 v1.2.1 起按 Apple 网格留白重做过的那张）
_icon = 'crates/dbmind-desktop/icons/icon.icns'
if os.path.exists(_icon):
    icon = _icon
