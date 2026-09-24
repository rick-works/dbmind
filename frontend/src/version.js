// 应用版本号（「设置 → 关于」页展示）。
//
// 唯一来源：仓库根 Cargo.toml 的 [workspace.package].version，由 vite.config.mjs
// 在构建时以 define 注入 __APP_VERSION__。本文件**不维护常量**。
//
// 为什么改成注入：这里原先写死 '1.0.0'（抬头还写着"由 desktop/scripts/bump-version.js
// 自动生成"，但那个脚本随 Tauri 迁移已经删掉了），而真实版本是 0.1.0 —— 结果
// 「关于」页显示 1.0.0、装出来的包却是 0.1.0，同一个东西两个版本号。
//
// typeof 判断用于没有构建注入的场景（如单测直接引入本模块），避免抛 ReferenceError。
export const APP_VERSION = typeof __APP_VERSION__ === 'string' ? __APP_VERSION__ : '0.0.0-dev'
