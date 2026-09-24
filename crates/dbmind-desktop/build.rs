fn main() {
    // 读同目录的 tauri.conf.json：生成上下文、按 bundle.icon 打进 Windows 图标资源。
    tauri_build::build()
}
