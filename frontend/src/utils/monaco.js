/**
 * Monaco 按需加载（首屏瘦身）。
 *
 * <p>此前 main.js 顶层 `import * as monaco from 'monaco-editor'`，vite 会把 monaco
 * 变成主包的静态依赖并在 index.html 生成 modulepreload —— 首屏无条件下载约 2.6MB
 * （gzip 680KB），占首屏体积的六成以上，而绝大多数会话不一定打开 SQL 编辑器。
 *
 * <p>这里把「加载 monaco + 注册 worker + 配置 loader」收敛成一个单例 Promise：
 * 首次调用时才动态 import，同一个会话内多个编辑器组件共享一次加载。
 *
 * <p><b>为什么这里是一串 contrib 导入，而不是一个 `import 'monaco-editor'`：</b>
 * monaco-editor 0.56 只提供两个 ESM 入口，二者都不理想：
 * <ul>
 *   <li>{@code esm/vs/editor/editor.api.js} —— 纯 API，<b>不含任何 contrib</b>；</li>
 *   <li>{@code esm/vs/editor/editor.main.js} —— 全部 contrib + 全部语言定义，
 *       连带 typescript / css / html 语言服务（约 8MB）。</li>
 * </ul>
 * 而<b>自动补全（suggest）、悬浮、查找、折叠、格式化动作全部属于 contrib</b>。
 * 早先 `import * as monaco from 'monaco-editor'` 走的是后者，补全正常；改成按需加载时
 * 只 import 了 editor.api，suggestController 从未注册 —— 表现是「编辑、语法高亮都正常，
 * 唯独输入时不弹提示」（即 SQL 编辑器「没有提示了」的根因）。
 *
 * <p>所以这里取折中：editor.api + 手工挑选 SQL 编辑真正用到的 contrib，
 * 既恢复补全，又不把无关语言服务拖进产物。
 */
import { allBuiltinFunctions } from './sqlCompletions'
// loader 用**静态**导入：@guolao/vue-monaco-editor（SqlQueryView/SqlCodeEditor 已静态
// 依赖）本身就静态导入了它，动态导入拆不出独立 chunk，只会触发打包器的
// INEFFECTIVE_DYNAMIC_IMPORT 警告（Vite 8/Rolldown 实测）。
// 不影响首屏瘦身 —— 本模块只被懒加载的编辑器视图引用，loader 随它们所在 chunk 走。
import loader from '@monaco-editor/loader'

let loading = null

export function ensureMonaco() {
  if (loading) return loading
  loading = (async () => {
    const [monacoMod, workerMod] = await Promise.all([
      import('monaco-editor/esm/vs/editor/editor.api'),
      import('monaco-editor/esm/vs/editor/editor.worker?worker')
    ])

    // 以下均为「副作用模块」：import 即完成注册（contribution / 语言 / 本地化文案），
    // 无需取返回值。必须在创建编辑器之前注册完，故这里统一 await。
    await Promise.all([
      // SQL 语言（Monarch 分词在主线程，不需要语言服务 worker）
      import('monaco-editor/esm/vs/languages/definitions/sql/register.js'),

      // ---- 编辑器 contrib（缺了它们对应功能静默消失）----
      // 基础编辑命令：光标移动、删除、撤销/重做
      import('monaco-editor/esm/vs/editor/browser/coreCommands.js'),
      // 内置文案本地化（补全/悬浮的无障碍标签与提示语）
      import('monaco-editor/esm/vs/editor/common/standaloneStrings.js'),
      // ★ 自动补全：缺失时 suggest 下拉永远不弹出
      import('monaco-editor/esm/vs/editor/contrib/suggest/browser/suggestController.js'),
      // 补全基础设施（suggestController 依赖的行内补全通道）
      import('monaco-editor/esm/vs/editor/contrib/inlineCompletions/browser/inlineCompletions.contribution.js'),
      // 代码片段插入：我们注册了带 ${1:table_name} 占位符的补全项，靠它展开
      import('monaco-editor/esm/vs/editor/contrib/snippet/browser/snippetController2.js'),
      // 悬浮提示
      import('monaco-editor/esm/vs/editor/contrib/hover/browser/hoverContribution.js'),
      // 括号匹配（editorOptions.matchBrackets）
      import('monaco-editor/esm/vs/editor/contrib/bracketMatching/browser/bracketMatching.js'),
      // 代码折叠（editorOptions.folding）
      import('monaco-editor/esm/vs/editor/contrib/folding/browser/folding.js'),
      // 查找/替换
      import('monaco-editor/esm/vs/editor/contrib/find/browser/findController.js'),
      // 注释切换
      import('monaco-editor/esm/vs/editor/contrib/comment/browser/comment.js'),
      // 格式化动作（Shift+Alt+F 触发我们注册的 DocumentFormattingEditProvider）
      import('monaco-editor/esm/vs/editor/contrib/format/browser/formatActions.js'),
      // 缩进调整
      import('monaco-editor/esm/vs/editor/contrib/indentation/browser/indentation.js'),
      // 同名标识符高亮
      import('monaco-editor/esm/vs/editor/contrib/wordHighlighter/browser/wordHighlighter.js'),
      // 多光标
      import('monaco-editor/esm/vs/editor/contrib/multicursor/browser/multicursor.js'),
      // 行操作（上下移动行、复制行）
      import('monaco-editor/esm/vs/editor/contrib/linesOperations/browser/linesOperations.js'),
      // 智能扩选
      import('monaco-editor/esm/vs/editor/contrib/smartSelect/browser/smartSelect.js'),
      // 光标位置撤销
      import('monaco-editor/esm/vs/editor/contrib/cursorUndo/browser/cursorUndo.js'),
      // 词级操作（Ctrl+Backspace 删词等）
      import('monaco-editor/esm/vs/editor/contrib/wordOperations/browser/wordOperations.js'),
      // 驼峰/下划线分词操作
      import('monaco-editor/esm/vs/editor/contrib/wordPartOperations/browser/wordPartOperations.js'),
      // 剪贴板
      import('monaco-editor/esm/vs/editor/contrib/clipboard/browser/clipboard.js')
    ])

    const EditorWorker = workerMod.default
    self.MonacoEnvironment = { getWorker: () => new EditorWorker() }
    // 本地加载（绿色版离线运行，不走 CDN）
    loader.config({ monaco: monacoMod })

    // 内置函数填进 SQL 分词器的 builtinFunctions 表 → 命中 `@builtinFunctions` 规则、
    // 得到 `predefined` token（数据库自带的主题规则给它独立于关键字的颜色）。
    // 为什么不走关键字合并：那样函数名与 SELECT 同色，用户要求区分开。
    // 分词器按语言全局生效，故取各方言函数的并集；monarch 的 ignoreCase 让大小写都对上。
    const sqlEntry = monacoMod.languages.getLanguages().find((l) => l.id === 'sql')
    if (sqlEntry) {
      const mod = await sqlEntry.loader()
      mod.language.builtinFunctions = [
        ...new Set([...(mod.language.builtinFunctions || []), ...allBuiltinFunctions().map((f) => f.toLowerCase())])
      ]
      monacoMod.languages.setMonarchTokensProvider('sql', mod.language)
    }

    // 自定义主题：基于内置主题，给 `predefined`（内置函数）与关键字**区分开**的颜色。
    // colors 必须给（空对象也行）—— 缺了它 monaco 解析 'editor.foreground' 直接抛错，
    // 主题切换失败会连带编辑器都挂不上。
    monacoMod.editor.defineTheme('dbmind-light', {
      base: 'vs', inherit: true,
      rules: [{ token: 'predefined', foreground: '795E26' }],
      colors: {}
    })
    monacoMod.editor.defineTheme('dbmind-dark', {
      base: 'vs-dark', inherit: true,
      rules: [{ token: 'predefined', foreground: 'DCDCAA' }],
      colors: {}
    })
    return monacoMod
  })()
  return loading
}
