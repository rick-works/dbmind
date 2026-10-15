const fs = require('fs');
const p = 'd:/develop/workspace/product/DBMind/frontend/src/locales/zh-CN.js';
let s = fs.readFileSync(p, 'utf8');
const rep = [
  ["upv.select': 'SELECT 查询'", "upv.select': 'SELECT 查询数据'"],
  ["upv.insert': 'INSERT 插入'", "upv.insert': 'INSERT 插入数据'"],
  ["upv.update': 'UPDATE 更新'", "upv.update': 'UPDATE 更新数据'"],
  ["upv.delete': 'DELETE 删除'", "upv.delete': 'DELETE 删除数据'"],
  ["upv.create': 'CREATE 创建'", "upv.create': 'CREATE 创建库/表'"],
  ["upv.drop': 'DROP 删除'", "upv.drop': 'DROP 删除库/表'"],
  ["upv.index': 'INDEX 索引'", "upv.index': 'INDEX 创建/删除索引'"],
  ["upv.alter': 'ALTER 修改'", "upv.alter': 'ALTER 修改表结构'"],
  ["upv.createView': 'CREATE VIEW 视图'", "upv.createView': 'CREATE VIEW 创建视图'"],
  ["upv.showView': 'SHOW VIEW 查看视图'", "upv.showView': 'SHOW VIEW 查看视图定义'"],
  ["upv.trigger': 'TRIGGER 触发器'", "upv.trigger': 'TRIGGER 管理触发器'"],
  ["upv.event': 'EVENT 事件'", "upv.event': 'EVENT 事件调度'"],
  ["upv.execute': 'EXECUTE 执行'", "upv.execute': 'EXECUTE 执行存储过程'"],
  ["upv.references': 'REFERENCES 外键'", "upv.references': 'REFERENCES 创建外键'"],
  ["upv.lockTables': 'LOCK TABLES 锁表'", "upv.lockTables': 'LOCK TABLES 显式锁表'"],
  ["upv.createTemporaryTables': 'CREATE TEMPORARY TABLES 临时表'", "upv.createTemporaryTables': 'CREATE TEMPORARY TABLES 创建临时表'"],
  ["upv.createRoutine': 'CREATE ROUTINE 创建例程'", "upv.createRoutine': 'CREATE ROUTINE 创建存储过程/函数'"],
  ["upv.alterRoutine': 'ALTER ROUTINE 修改例程'", "upv.alterRoutine': 'ALTER ROUTINE 改删存储过程/函数'"],
  ["upv.file': 'FILE 文件读写'", "upv.file': 'FILE 读写服务器文件'"],
  ["upv.truncate': 'TRUNCATE 清空'", "upv.truncate': 'TRUNCATE 清空表'"],
  ["upv.optimize': 'OPTIMIZE 优化'", "upv.optimize': 'OPTIMIZE 优化表'"],
  ["upv.show': 'SHOW 显示'", "upv.show': 'SHOW 查看列表'"],
  ["upv.viewDefinition': 'VIEW DEFINITION 查看定义'", "upv.viewDefinition': 'VIEW DEFINITION 查看对象定义'"],
];
let n = 0;
for (const [a, b] of rep) {
  if (s.includes(a)) { s = s.split(a).join(b); n++; } else { console.log('MISS: ' + a); }
}
fs.writeFileSync(p, s);
console.log('replaced ' + n + '/' + rep.length);
