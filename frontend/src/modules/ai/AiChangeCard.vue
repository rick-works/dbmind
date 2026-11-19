<template>
  <!-- 变更单卡片：风险色条 + 等宽 SQL + 撤销语句，一眼看清「点了会发生什么」。
       对话时间线里**就地渲染**（不堆在气泡底部），记录抽屉里也用同一张卡。 -->
  <div class="chg-card" :class="['is-' + card.risk, { 'is-skipped': card.skipped }]">
    <div class="chg-head">
      <span class="chg-risk">{{ riskLabel(card.risk) }}</span>
      <span class="chg-title" :title="card.title">{{ card.title }}</span>
      <span class="chg-state">{{ card.skipped ? $t('ai.skipped') : chgStatusLabel(card.status) }}</span>
    </div>
    <div class="chg-path" :title="pathText">{{ pathText }}</div>
    <pre class="chg-sql">{{ card.sql }}</pre>
    <div v-if="card.undoSql" class="chg-undo">↩ {{ card.undoSql }}</div>
    <div v-if="!card.skipped" class="chg-acts">
      <el-button v-if="card.status === 'pending'" size="small" type="primary"
                 :loading="busy" @click="$emit('allow')">
        {{ $t('ai.allow') }}
      </el-button>
      <el-button v-if="card.status === 'pending'" size="small" text @click="$emit('reject')">
        {{ $t('ai.chgReject') }}
      </el-button>
      <el-button v-if="card.status === 'pending'" size="small" text @click="$emit('skip')">
        {{ $t('ai.skip') }}
      </el-button>
      <el-button v-if="card.status === 'applied'" size="small" :loading="busy" @click="$emit('undo')">
        {{ $t('ai.chgUndo') }}
      </el-button>
    </div>
    <!-- 跳过过的单子：动作被收起，但如果 AI 又提到它（或用户回头想处理），
         必须有一个入口把卡「重新打开」，否则就是一张点不动的死卡 -->
    <div v-else class="chg-acts">
      <el-button size="small" text @click="$emit('unskip')">{{ $t('ai.chgResume') }}</el-button>
    </div>
  </div>
</template>

<script setup>
import { computed } from 'vue'
import { t, te } from '../../utils/i18n'
import { changePathOf } from '../../utils/sqlTarget'

const props = defineProps({
  card: { type: Object, required: true },
  /** 连接名：卡片上要显示「连接.库.模式.表」，光有 id 认不出是哪条库 */
  connName: { type: String, default: '' },
  /** 这张卡正在执行动作（按钮转圈），同一时刻只允许一张 */
  busy: { type: Boolean, default: false }
})

defineEmits(['allow', 'reject', 'skip', 'undo', 'unskip'])

const riskLabel = (risk) => (te('ai.risk.' + risk) ? t('ai.risk.' + risk) : risk)
const chgStatusLabel = (status) => (te('ai.chgStatus.' + status) ? t('ai.chgStatus.' + status) : status)
const pathText = computed(() => changePathOf(props.card, props.connName))
</script>

<style scoped>
.chg-card {
  border: 1px solid var(--dc-border);
  border-left: 3px solid var(--dc-primary);
  border-radius: 6px;
  background: var(--dc-bg-card);
  padding: 8px 10px;
}
.chg-card.is-medium { border-left-color: #d19a00; }
.chg-card.is-high { border-left-color: var(--dc-danger); }
/* 头部：标题最多两行（原来单行省略，长索引名整个被截成「…」），
   状态徽标不许被挤走（flex:none），窄面板下自动换行 */
.chg-head { display: flex; align-items: flex-start; flex-wrap: wrap; gap: 4px 8px; }
.chg-risk {
  flex: none;
  padding: 0 6px;
  border-radius: 4px;
  font-size: 11px;
  line-height: 18px;
  background: var(--dc-primary-soft);
  color: var(--dc-primary);
}
.chg-card.is-medium .chg-risk { background: rgba(209, 154, 0, .14); color: #a87900; }
.chg-card.is-high .chg-risk { background: rgba(220, 60, 60, .14); color: var(--dc-danger); }
.chg-title {
  flex: 1 1 auto; min-width: 0; font-size: 12.5px; font-weight: 600; color: var(--dc-text);
  overflow-wrap: anywhere;
  display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden;
}
.chg-state {
  flex: none; align-self: center;
  padding: 0 6px; border-radius: 4px; line-height: 18px;
  font-size: 11px; color: var(--dc-text-weak); background: var(--ai-inset-bg, var(--dc-bg-soft));
}
.chg-sql {
  margin: 6px 0 0;
  padding: 6px 8px;
  border-radius: 4px;
  background: var(--dc-bg-deep);
  color: var(--dc-text-mid);
  font-size: 12px;
  white-space: pre-wrap;
  word-break: break-all;
}
/* 定位路径 chip：与聚焦表 tag 同一副长相（描边小圆角），能一眼看出改的是哪张表 */
.chg-path {
  display: inline-flex; align-items: center; max-width: 100%;
  margin-top: 6px; padding: 0 6px; height: 19px; border-radius: 5px;
  border: 1px solid var(--dc-border); background: var(--dc-bg-soft);
  font-size: 11px; line-height: 19px; color: var(--dc-text-mid);
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
}
.chg-undo { margin-top: 6px; font-size: 11px; color: var(--dc-text-dim); word-break: break-all; }
.chg-acts { margin-top: 8px; display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
/* 跳过 = 这次不执行（单子仍留在「记录」里可再执行），整卡压暗一档，别和待处理混在一起 */
.chg-card.is-skipped { opacity: .55; }
</style>
