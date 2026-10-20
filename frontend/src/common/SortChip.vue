<template>
  <!-- 排序链胶囊（表预览 / SQL 结果 / NoSQL 文档三张网格共用）：
       按优先级从左到右列出 A ↑ → B ↓，与表头排序图标旁的序号 1/2/3 一一对应。
       · 点某一项 = 只移除那一列（后面的列自动前移，序号不会出现空号）
       · 「取消」= 清空全部并回到原始行序
       追加多列排序走**表头箭头**（直接点，不用按住 Shift），底栏不再放追加按钮。 -->
  <span v-if="keys.length" class="sort-chip" :title="$t('sqlq.sortChipTitle')">
    <span>{{ $t('sqlq.sortedBy') }}</span>
    <template v-for="(k, i) in keys" :key="k.col">
      <span class="sort-chip-it" :title="$t('sqlq.sortRemoveOne')" @click.stop="$emit('remove', k.col)">
        {{ k.col }} {{ k.dir === 'ASC' ? '↑' : '↓' }}
      </span>
      <span v-if="i < keys.length - 1" class="sort-chip-sep">→</span>
    </template>
    <span class="sort-chip-x" @click.stop="$emit('clear')">{{ $t('sqlq.clearSort') }}</span>
  </span>
</template>

<script setup>
defineProps({
  /** 当前排序链：[{ col, dir }]（按优先级） */
  keys: { type: Array, default: () => [] }
})
defineEmits(['remove', 'clear'])
</script>

<style scoped>
.sort-chip {
  display: inline-flex; align-items: center;
  margin-left: 8px; white-space: nowrap;
  font-size: calc(var(--grid-fs, 13px) - 1px);
  color: var(--dc-primary);
}
/* 每一项单独可点（只移除那一列） */
.sort-chip-it { cursor: pointer; }
.sort-chip-it:hover { text-decoration: underline; }
.sort-chip-sep { margin: 0 4px; color: var(--dc-text-dim); }
.sort-chip-x { margin-left: 8px; cursor: pointer; color: var(--dc-text-dim); }
.sort-chip-x:hover { color: var(--dc-primary); text-decoration: underline; }
</style>
