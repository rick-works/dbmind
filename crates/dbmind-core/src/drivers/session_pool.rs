//! agent 会话池：把「一个连接」映射到「一到多条物理会话」，并守住**全局配额**。
//!
//! 为什么需要它（第二件是**修缺陷**，不只是提速）：
//!
//! 1. **并发**：一个 sessionId 在宿主侧就是**一条** JDBC 连接。HTTP 壳是 axum +
//!    `spawn_blocking`，两个请求（两个标签页、或一条查询 + 一次结构浏览）会真的同时
//!    进来 —— 共用一个 sessionId 时它们只能排队，长查询会把别的操作一起堵住。
//! 2. **安全**：宿主为了让 `cancel` 能插队，把每个请求丢进线程池执行 ⇒ 同一个
//!    sessionId 上的两个请求会**两个线程同时用同一条 `java.sql.Connection`**，
//!    而 JDBC 明确不保证连接线程安全。池化之后每个槽位同一时刻只被一个请求持有。
//!
//! 三个刻意的性质：
//!
//! - **惰性**：只有真的出现并发时才开第二条物理连接。顺序使用时与从前完全一样（一条），
//!   不会一上来就占满上限 —— 连接数是数据库侧的稀缺资源，JDBC 授权也常按连接数算。
//! - **粘性**：顺序调用始终复用**上一次用过**的那条。会话级状态（临时表、`SET`、
//!   未提交事务）因此不会在语句之间跳连接 —— 这是池化最容易踩坏的地方。
//! - **有上限**，而且是三层取最小：类型声明（YAML 的 `maxConnections`）→ 连接级覆盖
//!   （`extra.maxConnections`）→ **全局配额**（`SessionBudget`）。声明 `singleConnectionPool`
//!   的类型上限恒为 **1**，于是它退化成「同一时刻只有一个请求」的串行语义。
//!
//! 全局配额为什么在内核侧执行（宿主也知道自己有幾条会话）：**只有内核知道一条会话
//! 此刻是不是空闲**（槽位的 `busy` 就是权威判断），而回收必须只挑空闲的 —— 关掉一条
//! 正在跑语句的会话，用户看到的是「连接已断开」。放内核侧还让它对四个宿主
//! （JDBC / Mongo / ES / Redis）与将来的原生驱动**一处生效**。

use crate::error::{DbMindError, ErrorCode, Result};
use crate::CancelToken;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

/// 等不到槽位时的轮询间隔：配额侧的空缺只能靠回收制造，轮询比复杂唤醒更省事。
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// 一个槽位（= 一条物理会话）。
#[derive(Debug)]
struct Slot {
    /// 是否正被某个请求持有
    busy: bool,
    /// 最后一次被归还的时刻（回收时挑最久未用的）
    last_used: Instant,
}

impl Slot {
    fn new() -> Self {
        Self {
            busy: true,
            last_used: Instant::now(),
        }
    }
}

#[derive(Debug, Default)]
struct PoolState {
    /// 泳道 → 槽位。泳道 = 「连接配置 + 只读性 + 会话亲和」（见驱动）。
    ///
    /// 槽位用 `Option` 而不是直接弹出：**下标是会话 id 的一部分**，
    /// 回收一条后若把后面的往前挪，其余会话的 id 就会与宿主侧对不上（串会话）。
    /// 留下空位并复用，下标就永远稳定。
    lanes: HashMap<String, Vec<Option<Slot>>>,
    /// 泳道 → 上次用过的槽位下标（粘性复用靠它）
    last: HashMap<String, usize>,
}

impl PoolState {
    fn slots(&mut self, lane: &str) -> &mut Vec<Option<Slot>> {
        self.lanes.entry(lane.to_string()).or_default()
    }

    /// 该泳道上活着的槽位数。
    fn count(&self, lane: &str) -> usize {
        self.lanes
            .get(lane)
            .map(|slots| slots.iter().filter(|slot| slot.is_some()).count())
            .unwrap_or(0)
    }

    /// 选一个空闲槽位：**上次用过的那条优先**（粘性），否则第一条空闲的。
    fn pick_free(&self, lane: &str) -> Option<usize> {
        let slots = self.lanes.get(lane)?;
        let preferred = self.last.get(lane).copied();
        if let Some(index) = preferred {
            if slots.get(index).is_some_and(is_free) {
                return Some(index);
            }
        }
        slots.iter().position(is_free)
    }

    /// 放一个槽位（优先复用被回收留下的空位）。
    fn place(&mut self, lane: &str) -> usize {
        let slots = self.slots(lane);
        if let Some(index) = slots.iter().position(|slot| slot.is_none()) {
            slots[index] = Some(Slot::new());
            return index;
        }
        slots.push(Some(Slot::new()));
        slots.len() - 1
    }

    fn get_mut(&mut self, lane: &str, index: usize) -> Option<&mut Slot> {
        self.lanes.get_mut(lane)?.get_mut(index)?.as_mut()
    }

    /// 全局最久未用的**空闲**槽位（回收用）。
    fn oldest_idle(&self) -> Option<(String, Instant)> {
        let mut best: Option<(String, Instant)> = None;
        for (lane, slots) in self.lanes.iter() {
            for (index, slot) in slots.iter().enumerate() {
                let Some(slot) = slot else { continue };
                if slot.busy {
                    continue;
                }
                let better = best
                    .as_ref()
                    .map(|(_, when)| slot.last_used < *when)
                    .unwrap_or(true);
                if better {
                    best = Some((session_id(lane, index), slot.last_used));
                }
            }
        }
        best
    }

    /// 摘掉某个会话的槽位（**仍空闲才摘**）。摘不到说明它刚被用起来了。
    fn remove_idle(&mut self, target: &str) -> bool {
        for (lane, slots) in self.lanes.iter_mut() {
            for (index, slot) in slots.iter_mut().enumerate() {
                if session_id(lane, index) != target {
                    continue;
                }
                match slot {
                    Some(current) if !current.busy => {
                        *slot = None;
                        return true;
                    }
                    // 找到了但正忙：不摘（回收绝不能打断在跑的语句）
                    _ => return false,
                }
            }
        }
        false
    }
}

fn is_free(slot: &Option<Slot>) -> bool {
    slot.as_ref().is_some_and(|slot| !slot.busy)
}

/// 排队等待期间被取消 —— 措辞与「跑到一半被取消」区分开，
/// 排查时一眼就能看出这次卡在哪一步（**还没有开始执行**）。
///
/// 两处共用这一句话，因为它们是同一条要求：
/// agent 侧等会话（`SessionPool::acquire_cancellable`）与原生侧等连接锁
/// （`sqlite::lock_connection`）。
pub(crate) fn cancelled_while_waiting() -> DbMindError {
    DbMindError::new(
        ErrorCode::QueryCanceled,
        "等待可用连接时被取消（这条语句还没有开始执行）",
    )
}

/// 全局会话配额：**跨连接、跨类型、跨宿主**的物理会话总数上限。
///
/// 由引擎在启动与设置变更时设置上限（`Store::KEY_SESSION_MAX_PER_HOST`）。
/// 上限为 0 表示不限制，交给类型/连接级上限。
pub struct SessionBudget {
    cap: AtomicUsize,
    /// 空闲回收时长（秒；0 = 不按空闲回收）。见 `sweep_idle`。
    idle_timeout_secs: AtomicUsize,
    /// 当前活着的槽位数（= 物理会话数）。**由池在增删槽位时维护**，
    /// 不靠遍历统计：遍历要锁别的池，而池可能在持锁时问预算（锁序会反过来）。
    live: AtomicUsize,
    pools: Mutex<Vec<Registration>>,
}

/// 关闭一条会话的方式。由驱动提供：只有它知道该找哪个宿主（并按宿主协议发 `disconnect`）。
pub type SessionCloser = Arc<dyn Fn(&str) + Send + Sync>;

/// 预算眼里的「一个会话池」：能回答「最久未用的**空闲**会话是谁」，也能被摘掉一条。
///
/// 两类实现：agent 的 [`SessionPool`]（多槽位泳道）与原生驱动的连接池
/// （按文件一条，见 `sqlite.rs`）。抽象它的原因：**回收必须只挑空闲的**，
/// 而「空闲」只有池自己知道 —— agent 侧是槽位的 `busy`，原生侧是连接句柄的锁有没有被持有。
pub trait SessionSource: Send + Sync {
    /// 最久未用的空闲会话（会话 id + 时刻）；没有空闲的返回 `None`。
    fn oldest_idle(&self) -> Option<(String, Instant)>;
    /// 摘掉指定会话（**仍空闲才摘**）。摘到返回 true。
    fn take_idle(&self, session_id: &str) -> bool;
}

/// agent 侧的池状态就是这样一个来源（锁一下自己的状态再委托）。
impl SessionSource for Mutex<PoolState> {
    fn oldest_idle(&self) -> Option<(String, Instant)> {
        self.lock().unwrap_or_else(|e| e.into_inner()).oldest_idle()
    }

    fn take_idle(&self, session_id: &str) -> bool {
        self.lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove_idle(session_id)
    }
}

struct Registration {
    pool: Weak<dyn SessionSource>,
    /// 关掉一条会话。由驱动提供（只有它知道该找哪个宿主）。
    closer: SessionCloser,
}

/// 回收时用的句柄对（池 + 关它的方式）。取出来之后就不再持有预算锁。
type Registered = (Weak<dyn SessionSource>, SessionCloser);

impl std::fmt::Debug for SessionBudget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionBudget")
            .field("cap", &self.cap())
            .field("live", &self.live())
            .finish()
    }
}

impl Default for SessionBudget {
    fn default() -> Self {
        Self::new(0)
    }
}

impl SessionBudget {
    pub fn new(cap: usize) -> Self {
        Self {
            cap: AtomicUsize::new(cap),
            idle_timeout_secs: AtomicUsize::new(0),
            live: AtomicUsize::new(0),
            pools: Mutex::new(Vec::new()),
        }
    }

    pub fn set_cap(&self, cap: usize) {
        self.cap.store(cap, Ordering::Relaxed);
    }

    pub fn cap(&self) -> usize {
        self.cap.load(Ordering::Relaxed)
    }

    /// 空闲回收时长（秒；0 = 不按空闲回收，只保留「配额满时回收」）。
    pub fn set_idle_timeout_secs(&self, secs: usize) {
        self.idle_timeout_secs.store(secs, Ordering::Relaxed);
    }

    pub fn idle_timeout_secs(&self) -> usize {
        self.idle_timeout_secs.load(Ordering::Relaxed)
    }

    /// 按空闲时长回收一轮：把所有**空闲**且闲置超过设置时长的会话还回数据库。
    ///
    /// 与「配额满时回收一条」是两件事：那个为了**腾地方**，这个为了**主动归还** ——
    /// 用户走了以后连接不该一直占着（数据库侧的连接数是共享资源）。
    /// 返回回收了几条。
    pub fn sweep_idle(&self) -> usize {
        let secs = self.idle_timeout_secs();
        if secs == 0 {
            return 0;
        }
        // 时钟基准异常（`now - ttl` 越界）时什么都不回收：宁可不做事，也别误关
        let Some(cutoff) = Instant::now().checked_sub(Duration::from_secs(secs as u64)) else {
            return 0;
        };
        self.sweep_older_than(cutoff)
    }

    /// 回收所有 `last_used` 早于 `cutoff` 的空闲会话。
    ///
    /// 每轮重新挑「全局最久未用的那条」：
    /// - 挑不到（没有空闲会话）⇒ 停；
    /// - 最久的那条也还没到年龄 ⇒ 停；
    /// - 摘不到（刚被别的线程用起来了）⇒ 再来一轮 —— 它已经不空闲，不会再被选中，
    ///   所以循环一定会收敛，不会空转。
    pub(crate) fn sweep_older_than(&self, cutoff: Instant) -> usize {
        let mut freed = 0;
        while let Some((when, index, session_id)) = self.oldest_idle_across() {
            if when > cutoff {
                break;
            }
            let registrations: Vec<Registered> = {
                let guard = self.lock();
                guard
                    .iter()
                    .map(|item| (item.pool.clone(), item.closer.clone()))
                    .collect()
            };
            let Some(source) = registrations.get(index).and_then(|(weak, _)| weak.upgrade()) else {
                break;
            };
            if !source.take_idle(&session_id) {
                continue;
            }
            self.release_one();
            tracing::debug!(
                target: "dbmind::agent",
                session = %session_id,
                live = self.live(),
                cap = self.cap(),
                "按空闲时长归还会话（交给驱动去关）"
            );
            if let Some((_, closer)) = registrations.get(index) {
                closer(&session_id);
            }
            freed += 1;
        }
        freed
    }

    /// 全局最久未用的空闲会话（`(时刻, 池序号, 会话 id)`）。
    ///
    /// 取句柄时短暂持预算锁，之后逐个锁池 —— 顺序固定，见 `reclaim_idle` 的说明。
    fn oldest_idle_across(&self) -> Option<(Instant, usize, String)> {
        let registrations: Vec<Registered> = {
            let guard = self.lock();
            guard
                .iter()
                .map(|item| (item.pool.clone(), item.closer.clone()))
                .collect()
        };
        let mut best: Option<(Instant, usize, String)> = None;
        for (index, (weak, _)) in registrations.iter().enumerate() {
            let Some(source) = weak.upgrade() else { continue };
            if let Some((session_id, when)) = source.oldest_idle() {
                let better = best.as_ref().map(|(time, _, _)| when < *time).unwrap_or(true);
                if better {
                    best = Some((when, index, session_id));
                }
            }
        }
        best
    }

    /// 当前活着的物理会话数。
    pub fn live(&self) -> usize {
        self.live.load(Ordering::Relaxed)
    }

    /// 把「类型/连接声明」的上限夹到全局配额之内。
    ///
    /// 夹紧而不是报错：单条连接的并发不该超过全局预算，否则一条连接自己就会
    /// 把预算吃光（别处一用就触发回收，来回抖动）。
    pub fn clamp(&self, declared: usize) -> usize {
        let cap = self.cap();
        if cap == 0 {
            declared.max(1)
        } else {
            declared.max(1).min(cap)
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Registration>> {
        self.pools.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 登记一个池（由池在构造时调用）。**公开给原生驱动**：它们的连接池同样要计入配额。
    pub fn register(&self, pool: Weak<dyn SessionSource>, closer: SessionCloser) {
        let mut guard = self.lock();
        guard.retain(|item| item.pool.strong_count() > 0);
        guard.push(Registration { pool, closer });
    }

    /// 申请一个会话名额（原子，可并发）。满员返回 false。
    pub(crate) fn try_reserve(&self) -> bool {
        if self.cap() == 0 {
            self.live.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        loop {
            let live = self.live.load(Ordering::Relaxed);
            if live >= self.cap() {
                return false;
            }
            if self
                .live
                .compare_exchange_weak(live, live + 1, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return true;
            }
        }
    }

    /// 归还一个名额：给「预留了却没用上」的路径用（原生驱动并发开连接时会遇到），
    /// 回收时也走它。
    pub(crate) fn release_one(&self) {
        let _ = self
            .live
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |live| {
                Some(live.saturating_sub(1))
            });
    }

    /// 回收一条**全局最久未用且空闲**的会话：把它从池里摘掉，并让宿主真的关掉它。
    ///
    /// 只挑空闲的：关掉正在跑语句的会话会让用户看到「连接已断开」；而配额要解决的
    /// 只是「别占着数据库连接不放」—— 空闲的那些才是不放的。
    ///
    /// **锁序是硬约束**：预算锁只在取句柄时短暂持有，之后逐个锁池。
    /// 池在持锁时会问预算（`try_reserve` 是原子操作，不取预算锁），反向持锁才是死锁，
    /// 所以这里绝不能持预算锁去锁池。
    ///
    /// 返回是否真的回收了一条。
    pub fn reclaim_idle(&self) -> bool {
        let Some((_, index, session_id)) = self.oldest_idle_across() else {
            tracing::debug!(
                target: "dbmind::agent",
                live = self.live(),
                cap = self.cap(),
                "没有可回收的空闲会话（都在用，或还没有会话）"
            );
            return false;
        };
        // 取到句柄（锁序见 `oldest_idle_across` 的说明），再拿关闭方式
        let registrations: Vec<Registered> = {
            let guard = self.lock();
            guard
                .iter()
                .map(|item| (item.pool.clone(), item.closer.clone()))
                .collect()
        };
        let Some(pool) = registrations.get(index).and_then(|(weak, _)| weak.upgrade()) else {
            return false;
        };
        if !pool.take_idle(&session_id) {
            // 扫过之后它被用起来了 —— 放弃这一条，让调用方重试（下一轮会挑别的）
            return false;
        }
        self.release_one();
        tracing::debug!(
            target: "dbmind::agent",
            session = %session_id,
            live = self.live(),
            cap = self.cap(),
            "按配额回收空闲会话（交给驱动去关）"
        );
        // 真正关掉：此刻手上没有任何锁（closer 会去调宿主）
        (registrations[index].1)(&session_id);
        true
    }
}

/// 会话池。`limit` 由**调用方**给出（它是数据：类型声明 → 连接覆盖 → 全局配额）。
#[derive(Debug)]
pub struct SessionPool {
    /// 用 `Arc` 是为了让预算能弱引用它（回收空闲会话时要反过来找到池）。
    state: Arc<Mutex<PoolState>>,
    available: Condvar,
    budget: Arc<SessionBudget>,
}

impl SessionPool {
    pub fn new(budget: Arc<SessionBudget>, closer: SessionCloser) -> Self {
        let state = Arc::new(Mutex::new(PoolState::default()));
        // 注册的是**同一个分配**（`Arc<T>` → `Arc<dyn Trait>` 的 unsizing 不改分配），
        // 所以这个 Weak 与下面存进 `self.state` 的那份同生共死。
        budget.register(Arc::downgrade(&(state.clone() as Arc<dyn SessionSource>)), closer);
        Self {
            state,
            available: Condvar::new(),
            budget,
        }
    }

    fn lock(&self) -> MutexGuard<'_, PoolState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 取一个槽位；`wait` 之内拿不到就报错（而不是无限期挂着）。
    ///
    /// 顺序调用只会用到同一个槽位；只有并发才会长出第二个 —— 所以「连接被占满」
    /// 是一件**能说清楚**的事，不该表现为一句含糊的超时。
    ///
    /// 这个入口不带取消令牌（结构浏览、自检、测试用）；查询走
    /// [`SessionPool::acquire_cancellable`]。
    pub fn acquire(&self, lane: &str, limit: usize, wait: Duration) -> Result<SlotGuard<'_>> {
        self.acquire_cancellable(lane, limit, wait, None)
    }

    /// 取一个槽位，并且**在等待期间也认取消令牌**。
    ///
    /// 为什么必须有它：单连接类型上「第二条查询」只能排队等前一条让出会话 ——
    /// 等待期间若不看令牌，用户点了「取消」也不会有反应，它会一直等到**拿到会话**
    /// （也就是前一条长查询跑完，或等满 `wait` 预算）。用户看到的是「点了取消没反应」，
    /// 这与「取消立刻生效」的承诺正好相反。
    ///
    /// 轮询间隔本来就是 [`POLL_INTERVAL`]（100ms），所以取消的响应也是这个量级。
    pub fn acquire_cancellable(
        &self,
        lane: &str,
        limit: usize,
        wait: Duration,
        cancel: Option<&CancelToken>,
    ) -> Result<SlotGuard<'_>> {
        let limit = limit.max(1);
        let deadline = Instant::now() + wait;
        loop {
            // 排队期间被取消的调用**不该再占用会话**：直接如实报「已取消」，
            // 而不是等它拿到会话、跑起来、再到行流上才发现被取消。
            if cancel.is_some_and(CancelToken::is_cancelled) {
                return Err(cancelled_while_waiting());
            }
            let mut state = self.lock();
            if let Some(index) = state.pick_free(lane) {
                return Ok(self.take(state, lane, index, limit));
            }
            let lane_full = state.count(lane) >= limit;
            // 还能开一条吗？除了本泳道的上限，还要过全局配额这一关
            if !lane_full && self.budget.try_reserve() {
                let index = state.place(lane);
                return Ok(self.take(state, lane, index, limit));
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.exhausted(lane, limit, lane_full));
            }
            // 配额用尽时先试着回收一条空闲会话（只可能在别的泳道/别的连接上）。
            // **必须先放开本池的锁**：回收要锁别的池，持锁去锁别人就是死锁。
            drop(state);
            if !lane_full {
                tracing::debug!(
                    target: "dbmind::agent",
                    lane,
                    live = self.budget.live(),
                    cap = self.budget.cap(),
                    "全局配额已满，尝试回收一条空闲会话"
                );
                if self.budget.reclaim_idle() {
                    continue; // 腾出配额了，立刻重试
                }
            }
            let state = self.lock();
            let _ = self.available.wait_timeout(state, remaining.min(POLL_INTERVAL));
        }
    }

    fn take<'a>(
        &'a self,
        mut state: MutexGuard<'a, PoolState>,
        lane: &str,
        index: usize,
        limit: usize,
    ) -> SlotGuard<'a> {
        if let Some(slot) = state.get_mut(lane, index) {
            slot.busy = true;
        }
        state.last.insert(lane.to_string(), index);
        let session_id = session_id(lane, index);
        // 排查池化问题时第一个想知道的就是「这次调用落到哪条物理会话」——
        // 顺序调用应始终是同一个 id，只有并发才会出现第二个。
        tracing::debug!(
            target: "dbmind::agent",
            lane,
            slot = index,
            limit,
            live = self.budget.live(),
            cap = self.budget.cap(),
            session = %session_id,
            "分配会话槽位"
        );
        SlotGuard {
            pool: self,
            lane: lane.to_string(),
            index,
            session_id,
        }
    }

    fn release(&self, lane: &str, index: usize) {
        {
            let mut state = self.lock();
            if let Some(slot) = state.get_mut(lane, index) {
                slot.busy = false;
                slot.last_used = Instant::now();
            }
            // 归还时更新粘性：下一次顺序调用仍然落在这一条上
            state.last.insert(lane.to_string(), index);
        }
        // 等的人可能在等配额（回收也会腾出来），所以唤醒所有等待者
        self.available.notify_all();
    }

    fn exhausted(&self, lane: &str, limit: usize, lane_full: bool) -> DbMindError {
        if lane_full {
            return DbMindError::new(
                ErrorCode::QueryTimeout,
                format!("等待可用连接超时：这条连接的并发上限是 {limit}"),
            )
            .with_detail(format!(
                "会话槽位（{lane}）都被占用中，可能是别的标签页正在执行长语句。\
                 并发上限由连接类型的池化声明决定（声明 singleConnectionPool 的类型上限为 1）。"
            ));
        }
        DbMindError::new(
            ErrorCode::QueryTimeout,
            format!(
                "等待可用连接超时：全局会话配额已满（{} / {}）",
                self.budget.live(),
                self.budget.cap()
            ),
        )
        .with_detail(
            "所有连接加起来的物理会话数触到了上限（设置项 session.maxPerHost）。\
             内核会优先回收**空闲**会话来腾位置；若所有会话都在执行语句，就只能等。\
             可以把上限调大，或减少同时打开的连接/页面。"
                .to_string(),
        )
    }
}

/// 槽位守卫：**drop 时归还**，所以提前 return / `?` 也不会漏还。
#[derive(Debug)]
pub struct SlotGuard<'a> {
    pool: &'a SessionPool,
    lane: String,
    index: usize,
    session_id: String,
}

impl SlotGuard<'_> {
    /// 这条槽位对应的 sessionId —— 宿主侧正是它决定用哪条物理连接。
    pub fn session_id(&self) -> &str {
        &self.session_id
    }
}

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        self.pool.release(&self.lane, self.index);
    }
}

/// sessionId 的形状：`<泳道>#<槽位>`。
///
/// 内核把它原样交给宿主当会话 id（宿主只当它是键），所以改形状不会破坏兼容；
/// 带上序号是为了让日志/宿主状态里的「这是第几条连接」一眼可读。
fn session_id(lane: &str, index: usize) -> String {
    format!("{lane}#{index}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    fn lane() -> &'static str {
        "postgresql|a|127.0.0.1|5432|app|-|false"
    }

    /// 无事发生的 closer（不关心回收时用）。
    fn noop() -> SessionCloser {
        Arc::new(|_: &str| {})
    }

    /// 被关掉的会话 id（回收时由 closer 记录）。
    type ClosedLog = Arc<StdMutex<Vec<String>>>;

    /// 记录被关掉的会话 id 的 closer。
    fn recorder() -> (ClosedLog, SessionCloser) {
        let closed: ClosedLog = Arc::new(StdMutex::new(Vec::new()));
        let sink = closed.clone();
        let closer: SessionCloser = Arc::new(move |id: &str| {
            sink.lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(id.to_string());
        });
        (closed, closer)
    }

    /// 不限制全局配额的池（**泳道上限**由每次 `acquire` 传）。
    ///
    /// 名字里刻意不叫 `pool(n)`：那个 `n` 是**全局配额**，不是泳道上限 ——
    /// 两者混起来会让「不同泳道互不影响」这种测试莫名其妙地失败（我踩过一次）。
    fn pool() -> SessionPool {
        pool_with_quota(0)
    }

    /// 带全局配额的池（0 = 不限制）。
    fn pool_with_quota(cap: usize) -> SessionPool {
        SessionPool::new(Arc::new(SessionBudget::new(cap)), noop())
    }

    /// 排队等会话时也要能被取消。
    ///
    /// 单连接类型上第二条查询只能等前一条让出会话；等待期间点「取消」必须**立刻**
    /// 有反应 —— 否则用户看到的是「点了取消没反应」，要等前一条跑完或等满预算。
    #[test]
    fn 排队等待时也能被取消() {
        let pool = pool();
        // 占住唯一的槽位
        let _held = pool
            .acquire(lane(), 1, Duration::from_millis(50))
            .expect("第一条应当拿到槽位");

        let token = CancelToken::new();
        let started = Instant::now();
        let outcome = std::thread::scope(|scope| {
            let handle =
                scope.spawn(|| pool.acquire_cancellable(lane(), 1, Duration::from_secs(10), Some(&token)));
            // 让它真的进到等待里，再取消
            std::thread::sleep(Duration::from_millis(120));
            token.cancel();
            handle.join().expect("等待线程不应 panic")
        });
        let waited = started.elapsed();

        let err = outcome.expect_err("被取消的等待不该拿到槽位");
        assert_eq!(err.code, ErrorCode::QueryCanceled, "排队被取消应报「已取消」");
        assert!(
            err.message.contains("等待可用连接"),
            "要说清是**排队时**被取消的（与跑到一半被取消区分开）：{}",
            err.message
        );
        assert!(
            waited < Duration::from_secs(2),
            "取消要立刻生效，不能等满 10 秒预算：实测 {waited:?}"
        );
    }

    /// 已经取消的令牌：**连槽位都不该占用**（更不该跑起来）。
    #[test]
    fn 已取消的令牌不会占用会话() {
        let pool = pool();
        let token = CancelToken::new();
        token.cancel();
        let err = pool
            .acquire_cancellable(lane(), 4, Duration::from_millis(10), Some(&token))
            .expect_err("已取消的调用不该拿到槽位");
        assert_eq!(err.code, ErrorCode::QueryCanceled);
    }

    /// 用默认上限（与 YAML 的默认一致）取一个槽位。
    fn grab<'a>(pool: &'a SessionPool, lane: &str, wait: Duration) -> Result<SlotGuard<'a>> {
        pool.acquire(lane, 4, wait)
    }

    #[test]
    fn 顺序调用始终复用同一条会话() {
        let pool = pool();
        let first = grab(&pool, lane(), Duration::from_secs(1)).unwrap();
        let id = first.session_id().to_string();
        drop(first);

        for _ in 0..5 {
            let guard = grab(&pool, lane(), Duration::from_secs(1)).unwrap();
            assert_eq!(
                guard.session_id(),
                id,
                "顺序调用不该换连接（会话级状态会因此丢失）"
            );
        }
        // 只长出一条：惰性
        assert_eq!(pool.lock().count(lane()), 1);
    }

    #[test]
    fn 并发时才长第二条且各自互不相同() {
        let pool = pool();
        let first = grab(&pool, lane(), Duration::from_secs(1)).unwrap();
        let second = grab(&pool, lane(), Duration::from_secs(1)).unwrap();
        assert_ne!(first.session_id(), second.session_id());
        assert_eq!(pool.lock().count(lane()), 2);

        // 归还后：下一次落在**最后归还**的那条上（粘性）
        let last = second.session_id().to_string();
        drop(second);
        let next = grab(&pool, lane(), Duration::from_secs(1)).unwrap();
        assert_eq!(next.session_id(), last);
    }

    #[test]
    fn 到达上限后等待归还而不是继续开() {
        let pool = Arc::new(pool());
        let held = pool.acquire(lane(), 2, Duration::from_secs(1)).unwrap();
        let also = pool.acquire(lane(), 2, Duration::from_secs(1)).unwrap();

        // 第三条必须等（上限 2）
        let waiter = {
            let pool = pool.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                let guard = pool.acquire(lane(), 2, Duration::from_secs(5)).unwrap();
                let waited = started.elapsed();
                let id = guard.session_id().to_string();
                (id, waited)
            })
        };
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(pool.lock().count(lane()), 2, "等待期间不该偷偷开第三条");

        assert_ne!(held.session_id(), also.session_id());
        let freed = also.session_id().to_string();
        drop(also);
        let (waited_id, waited) = waiter.join().unwrap();
        assert!(waited >= Duration::from_millis(100), "应当真的等过");
        // 拿到的是**刚归还的那一条**：粘性规则说「优先复用上一次用过的」，
        // 归还也算「用过」。这里要的是「等到了空闲槽位」，不是「必须回到 0 号」。
        assert_eq!(waited_id, freed, "应当复用刚归还的那一条");
        assert_eq!(pool.lock().count(lane()), 2, "等待者不该再开第三条");
    }

    #[test]
    fn 上限为_1_时退化成串行而不是报错() {
        let pool = Arc::new(pool());
        let held = pool.acquire(lane(), 1, Duration::from_secs(1)).unwrap();

        // 单连接类型：第二个请求**必须等**（它不可能拿到另一条连接）
        let waiter = {
            let pool = pool.clone();
            std::thread::spawn(move || {
                let guard = pool.acquire(lane(), 1, Duration::from_secs(5)).unwrap();
                let id = guard.session_id().to_string();
                drop(guard);
                id
            })
        };
        std::thread::sleep(Duration::from_millis(120));
        drop(held);
        let id = waiter.join().unwrap();
        assert_eq!(pool.lock().count(lane()), 1);
        assert_eq!(id, session_id(lane(), 0));
    }

    #[test]
    fn 等不到时给出可解释的错误而不是永久挂着() {
        let pool = pool();
        let _held = pool.acquire(lane(), 1, Duration::from_secs(1)).unwrap();
        let started = Instant::now();
        let err = pool.acquire(lane(), 1, Duration::from_millis(120)).unwrap_err();
        assert_eq!(err.code, ErrorCode::QueryTimeout);
        assert!(
            started.elapsed() >= Duration::from_millis(100),
            "应当等满预算再报错"
        );
        assert!(err.message.contains("并发上限"), "{}", err.message);
        assert!(
            err.detail
                .as_deref()
                .unwrap_or_default()
                .contains("singleConnectionPool"),
            "细节里要说清上限从哪来"
        );
    }

    #[test]
    fn 不同泳道互不影响() {
        let pool = pool();
        let a = pool.acquire("lane-a", 1, Duration::from_millis(50)).unwrap();
        // 另一条泳道（比如同连接的只读元数据会话、或另一个页面的会话）不该被它堵住
        let b = pool.acquire("lane-b", 1, Duration::from_millis(50)).unwrap();
        assert_ne!(a.session_id(), b.session_id());
    }

    #[test]
    fn 归还之后可以再次取用() {
        let pool = pool();
        for _ in 0..3 {
            let guard = pool.acquire(lane(), 1, Duration::from_millis(50)).unwrap();
            assert_eq!(guard.session_id(), session_id(lane(), 0));
        }
    }

    /// 上限是**每次调用传入的数据**，所以它会变。用户把它调小时：
    /// 不再新增槽位，但**已有的仍可复用** —— 不能因为改了个设置就把别处正在用的
    /// 连接掐掉（那会把在跑的语句变成「连接已断开」）。
    #[test]
    fn 上限调小后不再新增但已有槽位仍可复用() {
        let pool = pool();
        let a = grab(&pool, lane(), Duration::from_secs(1)).unwrap();
        let b = grab(&pool, lane(), Duration::from_secs(1)).unwrap();
        let a_id = a.session_id().to_string();
        drop(a);
        drop(b);
        assert_eq!(pool.lock().count(lane()), 2);

        // 上限改成 1：两条已存在的槽位仍然可复用（不该被销毁）
        let one = pool.acquire(lane(), 1, Duration::from_secs(1)).unwrap();
        let two = pool.acquire(lane(), 1, Duration::from_secs(1)).unwrap();
        assert_eq!(pool.lock().count(lane()), 2, "调小上限不该销毁已有槽位");

        // 两条都占着、而上限是 1 ⇒ 第三个请求只能等（不再新增）
        let err = pool.acquire(lane(), 1, Duration::from_millis(120)).unwrap_err();
        assert_eq!(err.code, ErrorCode::QueryTimeout);
        assert_eq!(pool.lock().count(lane()), 2, "仍不该新增（上限已调小）");
        assert!(a_id.ends_with("#0"), "会话 id 的形状保持稳定：{a_id}");
        drop(one);
        drop(two);
    }

    /// 全局配额是最后一道闸：所有连接加起来不能超过它。
    #[test]
    fn 全局配额把跨泳道的总数封住() {
        let budget = Arc::new(SessionBudget::new(2));
        let pool = SessionPool::new(budget.clone(), noop());

        let a = pool.acquire("lane-a", 4, Duration::from_secs(1)).unwrap();
        let b = pool.acquire("lane-b", 4, Duration::from_secs(1)).unwrap();
        assert_eq!(budget.live(), 2, "两条泳道各一条 ⇒ 配额用满");

        // 第三条拿不到配额：即使本泳道上限是 4，也只能等
        let err = pool.acquire("lane-c", 4, Duration::from_millis(150)).unwrap_err();
        assert_eq!(err.code, ErrorCode::QueryTimeout);
        assert!(err.message.contains("配额"), "{}", err.message);
        assert!(
            err.detail
                .as_deref()
                .unwrap_or_default()
                .contains("session.maxPerHost"),
            "要说清配额从哪来"
        );
        drop(a);
        drop(b);
    }

    /// 配额满时**回收空闲会话**腾位置；回收后总数不变。
    #[test]
    fn 配额满时回收最久未用的空闲会话() {
        let (closed, closer) = recorder();
        let budget = Arc::new(SessionBudget::new(2));
        let pool = SessionPool::new(budget.clone(), closer);

        let first = pool.acquire("lane-a", 1, Duration::from_secs(1)).unwrap();
        let first_id = first.session_id().to_string();
        drop(first);
        std::thread::sleep(Duration::from_millis(5));
        let second = pool.acquire("lane-b", 1, Duration::from_secs(1)).unwrap();
        let second_id = second.session_id().to_string();
        drop(second);
        assert_eq!(budget.live(), 2);

        // 第三条（新泳道）：配额已满 ⇒ 回收最久未用的那条（lane-a 先归还 ⇒ 更旧）
        let third = pool.acquire("lane-c", 1, Duration::from_secs(2)).unwrap();
        assert_ne!(third.session_id(), first_id);
        assert_eq!(budget.live(), 2, "回收一条又开一条，总数仍是 2");
        assert_eq!(
            closed.lock().unwrap_or_else(|e| e.into_inner()).as_slice(),
            [first_id.as_str()],
            "应当关掉的正是最久未用的那条"
        );
        assert!(!closed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&second_id));
    }

    /// 回收只挑**空闲**会话：正在跑语句的那条绝不能被关掉。
    #[test]
    fn 回收绝不碰正在使用的会话() {
        let (closed, closer) = recorder();
        let budget = Arc::new(SessionBudget::new(2));
        let pool = SessionPool::new(budget.clone(), closer);

        // lane-a 一直持有（忙），lane-b 用完归还（空闲）
        let busy = pool.acquire("lane-a", 1, Duration::from_secs(1)).unwrap();
        let idle = pool.acquire("lane-b", 1, Duration::from_secs(1)).unwrap();
        let idle_id = idle.session_id().to_string();
        drop(idle);

        let third = pool.acquire("lane-c", 1, Duration::from_secs(2)).unwrap();
        assert_eq!(
            closed.lock().unwrap_or_else(|e| e.into_inner()).as_slice(),
            [idle_id.as_str()],
            "应当关掉空闲的那条，而不是还在用的那条"
        );
        assert!(!closed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&busy.session_id().to_string()));
        assert_eq!(budget.live(), 2);
        drop(third);
        drop(busy);
    }

    /// 空闲超时回收：与配额**无关**，到点就把空闲会话还回去
    ///（用户走了以后，连接不该一直占着数据库）。
    #[test]
    fn 空闲超时会把会话还回去() {
        let (closed, closer) = recorder();
        let budget = Arc::new(SessionBudget::new(0));
        let pool = SessionPool::new(budget.clone(), closer);

        let held = grab(&pool, lane(), Duration::from_secs(1)).unwrap();
        let id = held.session_id().to_string();
        drop(held);
        assert_eq!(budget.live(), 1);

        // 还没到年龄：什么都不动
        budget.set_idle_timeout_secs(600);
        assert_eq!(budget.sweep_idle(), 0);
        assert_eq!(budget.live(), 1, "不该把还年轻的会话收走");

        // 用一个「未来」的截止时刻，等价于「它已经闲置很久了」
        let cutoff = Instant::now() + Duration::from_secs(1);
        assert_eq!(budget.sweep_older_than(cutoff), 1);
        assert_eq!(budget.live(), 0);
        assert_eq!(
            closed.lock().unwrap_or_else(|e| e.into_inner()).as_slice(),
            [id.as_str()]
        );
    }

    /// 空闲回收**只碰空闲的**：正在跑语句的会话即使「很老」也不会被收走。
    #[test]
    fn 空闲回收不碰正在使用的会话() {
        let (closed, closer) = recorder();
        let budget = Arc::new(SessionBudget::new(0));
        let pool = SessionPool::new(budget.clone(), closer);

        let busy = pool.acquire("lane-a", 1, Duration::from_secs(1)).unwrap();
        let idle = pool.acquire("lane-b", 1, Duration::from_secs(1)).unwrap();
        let idle_id = idle.session_id().to_string();
        drop(idle);

        let cutoff = Instant::now() + Duration::from_secs(1);
        assert_eq!(budget.sweep_older_than(cutoff), 1, "只该收走空闲的那条");
        assert_eq!(
            closed.lock().unwrap_or_else(|e| e.into_inner()).as_slice(),
            [idle_id.as_str()]
        );
        assert_eq!(budget.live(), 1, "忙的那条还在");
        drop(busy);
    }

    /// 时长为 0 = **不按空闲回收**（默认行为，别把老行为悄悄改掉）。
    #[test]
    fn 空闲时长为_0_时不做任何回收() {
        let budget = Arc::new(SessionBudget::new(0));
        let pool = SessionPool::new(budget.clone(), noop());
        let guard = grab(&pool, lane(), Duration::from_secs(1)).unwrap();
        drop(guard);

        assert_eq!(budget.idle_timeout_secs(), 0);
        assert_eq!(budget.sweep_idle(), 0);
        assert_eq!(budget.live(), 1, "默认不回收：会话留着");
    }

    /// 配额为 0 表示不限制（默认行为，别把老行为改掉）。
    #[test]
    fn 配额为_0_表示不限制() {
        let budget = SessionBudget::new(0);
        assert!(budget.try_reserve());
        assert!(budget.try_reserve());
        assert!(budget.try_reserve());
        assert_eq!(budget.live(), 3);
        assert_eq!(budget.clamp(4), 4);
        assert_eq!(budget.clamp(0), 1, "上限至少要 1 条");
    }

    /// 上限调小之后：已有会话不受影响，只是不再新增（`clamp` 只管新算出的上限）。
    #[test]
    fn 配额可以热调且不破坏已有会话() {
        let budget = SessionBudget::new(4);
        assert_eq!(budget.clamp(8), 4, "声明超过配额时按配额夹住");
        assert!(budget.try_reserve() && budget.try_reserve() && budget.try_reserve());
        assert!(budget.try_reserve());

        // 调小到 2：已占的 4 个不会被强杀（没有空闲的也就回收不出来），
        // 但新名额再也批不出来
        budget.set_cap(2);
        assert!(!budget.try_reserve());
        assert_eq!(budget.live(), 4);
        assert_eq!(budget.clamp(8), 2);
    }
}
