/**
 * `mergeAlterRuns` 的回归断言（`npm run guard` 会跑）。
 *
 * 为什么必须有这一条：它一旦静默失效，界面生成的是**错 SQL**（用户看到
 * 「ALTER TABLE `t` `t`」这种），而且只在真机点「保存修改」时才暴露。
 * 历史的坑：正则里 IDENT_PART 写成了捕获组 `(…)`，于是 `m[2]` 取到表名而不是子句，
 * 所有合并都变成 `ALTER TABLE <表> <表>` —— 断言缺失时一路绿到底。
 */
import { mergeAlterRuns } from '../src/modules/data/alterMerge.js'

const eq = (a, b, label) => {
  const ok = JSON.stringify(a) === JSON.stringify(b)
  if (!ok) {
    console.error(`✗ ${label}\n  期望: ${JSON.stringify(b)}\n  实际: ${JSON.stringify(a)}`)
    process.exitCode = 1
  } else {
    console.log(`✓ ${label}`)
  }
}

// ① 单条 ALTER：子句必须完整保留（曾经丢掉 MODIFY COLUMN 只剩表名）
eq(
  mergeAlterRuns({ table: '`t`', kind: 'mysql', parts: ['ALTER TABLE `t` MODIFY COLUMN `c` INT NOT NULL COMMENT \'x\''] }),
  ['ALTER TABLE `t` MODIFY COLUMN `c` INT NOT NULL COMMENT \'x\''],
  'MySQL 单条 MODIFY 原样保留'
)

// ② 多条同一张表：并成一条，子句用逗号连接
eq(
  mergeAlterRuns({
    table: '`t`',
    kind: 'mysql',
    parts: ['ALTER TABLE `t` MODIFY COLUMN `a` INT NOT NULL', 'ALTER TABLE `t` ADD COLUMN `b` VARCHAR(10) NULL']
  }),
  ['ALTER TABLE `t` MODIFY COLUMN `a` INT NOT NULL, ADD COLUMN `b` VARCHAR(10) NULL'],
  '同表多条并成一条 ALTER'
)

// ③ 别的表 / 非 ALTER 形状的语句：不参与合并，顺序不变
eq(
  mergeAlterRuns({
    table: '`t`',
    kind: 'oracle',
    parts: ['DROP INDEX `idx`', 'ALTER TABLE `t` MODIFY (`a` NUMBER(10))', 'COMMENT ON TABLE `t` IS \'c\'']
  }),
  ['DROP INDEX `idx`', 'ALTER TABLE `t` MODIFY (`a` NUMBER(10))', 'COMMENT ON TABLE `t` IS \'c\''],
  '非 ALTER / 其它语句保持独立'
)

// ④ 中文表名（JS 的 \w 不含中文，历史坑）仍能合并
eq(
  mergeAlterRuns({
    table: '`订单`',
    kind: 'mysql',
    parts: ['ALTER TABLE `订单` MODIFY COLUMN `金额` DECIMAL(10,2) NOT NULL', 'ALTER TABLE `订单` ADD COLUMN `备注` TEXT NULL']
  }),
  ['ALTER TABLE `订单` MODIFY COLUMN `金额` DECIMAL(10,2) NOT NULL, ADD COLUMN `备注` TEXT NULL'],
  '中文标识符参与合并'
)

// ⑤ SQL Server 的两段带引号表名
eq(
  mergeAlterRuns({
    table: '[dbo].[t]',
    kind: 'mssql',
    parts: ['ALTER TABLE [dbo].[t] ADD [c] INT NULL', 'ALTER TABLE [dbo].[t] ADD [d] INT NULL']
  }),
  ['ALTER TABLE [dbo].[t] ADD [c] INT NULL, ADD [d] INT NULL'],
  'SQL Server 两段式表名'
)

if (!process.exitCode) console.log('alterMerge 断言全部通过')
