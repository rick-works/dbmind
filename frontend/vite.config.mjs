import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { fileURLToPath, URL } from 'node:url'
import { readFileSync } from 'node:fs'

// 版本号唯一来源：仓库根 Cargo.toml 的 [workspace.package].version
// （tauri.conf.json 的 version 与它一致，release.ps1 打包前会校验那两处）。
// 前端不再自己维护版本常量：src/version.js 原先手写死 '1.0.0'，而真实版本是 0.1.0，
// 于是「关于」页显示的版本和安装包的实际版本长期对不上。
const cargoVersion = (() => {
  try {
    const toml = readFileSync(fileURLToPath(new URL('../Cargo.toml', import.meta.url)), 'utf8')
    return /\[workspace\.package\][\s\S]*?version\s*=\s*"([^"]+)"/.exec(toml)?.[1] || '0.0.0'
  } catch {
    return '0.0.0'
  }
})()

// 说明：本配置为 .mjs（原生 ESM）。
// Vite 的 configLoader 未来将以 native 为默认，届时把 ESM 语法写进 .js（按 CommonJS 加载）
// 会告警；直接使用 .mjs 可消除该告警，也让 import.meta.url 可用。
export default defineConfig({
  plugins: [vue()],
  // 构建时把版本号注入成字面量：src/version.js 读 __APP_VERSION__（版本不再手写在源码里）
  define: { __APP_VERSION__: JSON.stringify(cargoVersion) },
  base: './',
  resolve: {
    alias: {
      // monaco-editor 0.5x 起 package.json 只声明了 "." 一个 exports 入口，
      // 其内部路径（esm/vs/...）会被 exports 规则拦下，导致
      // 「monaco-editor/esm/vs/editor/editor.worker?worker」这类 worker 导入直接解析失败。
      // Vite 的 alias 在 exports 解析之前生效，把该前缀指向物理目录即可绕开限制；
      // 裸导入 'monaco-editor' 不受影响（走 exports 的 "." -> esm/vs/index.js）。
      'monaco-editor/esm': fileURLToPath(new URL('./node_modules/monaco-editor/esm', import.meta.url))
    }
  },
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://localhost:20361',
        changeOrigin: true
      }
    }
  },
  build: {
    outDir: 'dist',
    // 默认虽为 true，但这里显式写出来：dist 是「先清空再产出」，
    // 否则每轮构建都会在 assets 里再叠一份带新 hash 的副本，手工拷到后端 static 后越滚越大。
    emptyOutDir: true,
    // 这个阈值只用于捕捉「意外膨胀」（比如又把某个整包/worker 塞回首屏），不必追求小 chunk。
    // 当前最大 chunk 是 monaco 本体（约 4.3MB 未压缩，已单独成 chunk），故设为略高于它；
    // 真正衡量拆包效果的是 assets 里 js 总量与首屏 index chunk 大小。
    chunkSizeWarningLimit: 4500,
    rollupOptions: {
      output: {
        // 产物名统一加 v3- 前缀：真机发生过「浏览器缓存了被写坏的旧 body（构建窗口期
        // 拿到半截/回落 HTML），URL 不变就永远命中缓存 → hasError is not a function 白屏」。
        // 换前缀 = 所有 URL 全新，任何旧缓存（包括 immutable）都失效，普通刷新即可恢复。
        // 服务端已配套：assets/ 下不存在的文件一律 404，不再回落 index.html。
        chunkFileNames: 'assets/v3-[name]-[hash].js',
        entryFileNames: 'assets/v3-[name]-[hash].js',
        assetFileNames: 'assets/v3-[name]-[hash][extname]',
        // Vite 8 起默认打包器换为 Rolldown：manualChunks 只接受函数形式，
        // 原先的「对象映射」写法会直接报 Invalid type: Expected Function but received Object。
        // 注意 @element-plus/icons-vue 的模块 id 同样含 "element-plus" 子串，故一个判断即可覆盖。
        // monaco 刻意不做分组：强制成独立 chunk 后，rollup 会把「动态 import 的
        // 预加载 helper」塞进这个最大的公共块，于是 index 等含动态 import 的 chunk
        // 都会静态引用它，2.6MB 的 monaco 又被拽回首屏（index.html 出现其 modulepreload）。
        // 不分组后 monaco 只作为 utils/monaco.js 的异步依赖出现，首屏不再下载。
        //
        // [临时诊断·待定] 暂时关掉强制分组：Vite 8 的打包器 Rolldown 生成的产物里，
        // CJS 互操作助手 __commonJSMin 只有调用、没有定义（element / ErDiagramView 两个
        // chunk 分别调用 10 次 / 290 次，全项目 0 处定义），运行时报
        // 「__commonJSMin is not a function」整页白屏。怀疑是强制分组把 CJS 包
        // （async-validator / dagre）与它的运行时助手拆到了不同 chunk，先验证不分组是否正常。
        // manualChunks(id) {
        //   if (id.includes('element-plus')) return 'element'
        // }
      }
    }
  },
  optimizeDeps: {
    include: ['@guolao/vue-monaco-editor']
  }
})
