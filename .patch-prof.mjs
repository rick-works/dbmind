// 一次性补丁：给 export_query 加「取数 / 写盘」耗时剖析（DBMIND_EXPORT_PROFILE=1 时输出）。
import { readFileSync, writeFileSync } from 'node:fs'

const F = 'crates/dbmind-web/src/api/export.rs'
let s = readFileSync(F, 'utf8').replace(/\r\n/g, '\n')

const edits = [
  {
    // ① 循环前：开关与累加器
    o: `    let mut next_report = REPORT_ROWS;
    let mut next_log = LOG_ROWS;
    loop {`,
    n: `    let mut next_report = REPORT_ROWS;
    let mut next_log = LOG_ROWS;
    // 耗时剖析：设 DBMIND_EXPORT_PROFILE=1 时把「取数」与「写盘」的净耗时打到 stderr。
    // 默认零开销（只读一次环境变量），平时不打印；要判断"慢在哪一段"时直接开它。
    // 做它的原因：页大小实验只能说明"每页固定开销已接近 0"，但分不清剩下的是
    // 宿主取数还是内核写盘 —— 这两者对应的优化方向完全不同。
    let profile = std::env::var_os("DBMIND_EXPORT_PROFILE").is_some();
    let mut fetch_ms = 0f64;
    let mut write_ms = 0f64;
    loop {`,
  },
  {
    // ② 取数计时
    o: `        let result = run_sql_in(state, id, &req.database, sql, page_size as usize).await?;
        let got = result.rows.len() as u64;`,
    n: `        let t_fetch = std::time::Instant::now();
        let result = run_sql_in(state, id, &req.database, sql, page_size as usize).await?;
        if profile {
            fetch_ms += t_fetch.elapsed().as_secs_f64() * 1000.0;
        }
        let got = result.rows.len() as u64;`,
  },
  {
    // ③ 写盘计时：起
    o: `        // 逐行写盘，并且**页内也上报**：以前只在整页写完后更新一次，界面上只能 5000 一跳。
        for row in &result.rows {`,
    n: `        // 逐行写盘，并且**页内也上报**：以前只在整页写完后更新一次，界面上只能 5000 一跳。
        let t_write = std::time::Instant::now();
        for row in &result.rows {`,
  },
  {
    // ④ 写盘计时：止
    o: `        page += 1;
        if let Some(task) = task {
            task.set_done(written);`,
    n: `        if profile {
            write_ms += t_write.elapsed().as_secs_f64() * 1000.0;
        }
        page += 1;
        if let Some(task) = task {
            task.set_done(written);`,
  },
  {
    // ⑤ 收尾时打一行汇总
    o: `    sink.finish().map_err(XError::internal)?;`,
    n: `    let t_finish = std::time::Instant::now();
    sink.finish().map_err(XError::internal)?;
    if profile {
        let busy = fetch_ms + write_ms;
        let finish_ms = t_finish.elapsed().as_secs_f64() * 1000.0;
        let rate = if busy > 0.0 { written as f64 / (busy / 1000.0) } else { 0.0 };
        let share = if busy + finish_ms > 0.0 { fetch_ms / (busy + finish_ms) * 100.0 } else { 0.0 };
        eprintln!(
            "[export-profile] 行={written} 页={page} 取数={fetch_ms:.0}ms 写盘={write_ms:.0}ms 收尾={finish_ms:.0}ms 取数占比={share:.0}% 净速={rate:.0} 行/秒"
        );
    }`,
  },
]

let miss = 0
for (const e of edits) {
  const o = e.o
  const n = e.n
  if (s.includes(o)) {
    s = s.replace(o, n)
    console.log('  ✓ ' + o.trim().split('\n')[0].slice(0, 46))
  } else {
    miss++
    console.log('  ✗ MISS: ' + o.trim().split('\n')[0].slice(0, 46))
  }
}

if (miss === 0) {
  writeFileSync(F, s.replace(/\n/g, '\r\n'), 'utf8')
  console.log('  已写入 ✓')
} else {
  console.log('  ✗ 有 MISS，未写入')
}
