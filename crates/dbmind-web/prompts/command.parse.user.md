把指令解析成操作计划，只输出 JSON（无解释、无代码块）。

动作 type 只能用这些：
open_query(sql) / run_sql(sql, needConfirm) / export_table(table, format=csv|excel) /
open_panel(panel=ai|patrol|governance|settings|compare|sync) / analyze_table(table) /
create_index(table, ddl, needConfirm=true) / search(keyword) / answer(text)

输出结构：{"intent":"意图","actions":[{"type":"...","params":{},"needConfirm":false}],"summary":"给用户的一句话"}

建表/改表/删表/写数据必须 needConfirm=true；表名列名只取上下文，不臆造；
映射不到就 answer；一条指令可按顺序拆成多个动作。

{{context}}{{tables}}### 用户指令
{{command}}
