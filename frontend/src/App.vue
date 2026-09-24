<template>
  <el-config-provider :locale="epLocale">
    <router-view />
  </el-config-provider>
</template>

<script setup>
import { computed } from 'vue'
import zhCn from 'element-plus/es/locale/lang/zh-cn'
import en from 'element-plus/es/locale/lang/en'
import { locale } from './utils/i18n'

// Element Plus 自带组件的文案（分页器、日期选择器、表格空态、确认框按钮…）
// 与业务文案是**两套**东西：业务文案走 utils/i18n 的字典，这里必须同步切过去，
// 否则切到英文后分页器、"暂无数据"这些还留着中文 —— 界面上就像"只切了一半"。
// 原来这里的 `:locale="zhCn"` 是写死的，等于把语言永久钉在中文上。
const epLocale = computed(() => (locale.value === 'en-US' ? en : zhCn))
</script>
