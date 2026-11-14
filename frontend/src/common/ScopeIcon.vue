<template>
  <!-- 「范围」类下拉的前缀图标 —— 与**左侧对象树里同一个节点**长得完全一样：
       颜色说类型族（库/模式/目录=琥珀、表=蓝、视图=绿），形状说层级
       （有模式层的类型「库」是文件夹，没有模式层时「库」本身就是 Coin，模式恒为 Coin）。
       规则出处是 MainView 树模板的 v-else-if 图标链，改那里必须同步这里，否则
       同一个东西在树上和下拉里长得不一样（用户实测反馈过）。 -->
  <el-icon class="scope-ic" :color="color"><component :is="icon" /></el-icon>
</template>

<script setup>
import { computed } from 'vue'
import { Coin, Collection, Folder, Grid, View } from '@element-plus/icons-vue'
import { schemaLevelOf } from '../types'

const props = defineProps({
  /** db | schema | catalog | table | view */
  kind: { type: String, default: 'db' },
  /** 连接类型（如 MYSQL / POSTGRESQL）：只有「库」这一级需要它来判有没有模式层 */
  connType: { type: String, default: '' },
})

const icon = computed(() => {
  if (props.kind === 'schema') return Coin
  if (props.kind === 'catalog') return Collection
  if (props.kind === 'table') return Grid
  if (props.kind === 'view') return View
  // 「库」：PG / SQL Server / DB2 这类库里还有一层模式 → 文件夹；其余（MySQL / SQLite / CH…）
  // 库本身就是模式 → Coin（与树里收起态的 Folder 对应；树展开时画 FolderOpened，下拉没有展开态）
  return schemaLevelOf(props.connType) === 'schema' ? Folder : Coin
})

const color = computed(() => {
  if (props.kind === 'table') return '#4f8cff'
  if (props.kind === 'view') return '#3ddc97'
  return '#f5b34d'
})
</script>
