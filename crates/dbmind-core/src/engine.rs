//! `DbMindEngine`：内核的**唯一门面**。
//!
//! 所有壳（桌面 / Web / CLI / MCP / AI）都只调用这一层，因此：
//! - 行为一致：安全策略、超时、取消、历史记录不会因壳而异；
//! - 新增一个壳 = 写薄薄的 I/O 适配，不需要懂数据库；
//! - 测试只需针对这一层，覆盖即可代表全部入口。

use crate::cancel::CancelRegistry;
use crate::drivers::{DriverRegistry, QueryCall};
use crate::error::{DbMindError, ErrorCode, Result};
use crate::paths;
use crate::safety::SafetyPolicy;
use crate::storage::Store;
use crate::types::*;
use crate::{all_descriptors, ConnectionKind, RuntimeMode, TypeDescriptor};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Instant;

/// 全局会话配额的默认值（每个宿主进程最多持有多少条物理会话）。
///
/// 取 32 的理由：正常使用（几个连接 × 每个几条）离它很远，而它又确实拦得住
/// 「连接数 × 并发上限」在数据库侧失控 —— 很多服务的 `max_connections` 只有几十。
/// 0 表示不限制（交给类型/连接级上限）。
pub const DEFAULT_SESSION_MAX_PER_HOST: usize = 32;

/// 空闲会话清理线程的扫描间隔。
///
/// 10 秒的取舍：比「分钟级」的空闲时长灵敏得多，又完全谈不上开销 ——
/// 一次扫描只是遍历几个池的状态，而且多数时候一条空闲会话都没有。
const SESSION_SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(10);

pub struct DbMindEngine {
    store: Arc<Store>,
    drivers: DriverRegistry,
    cancels: CancelRegistry,
    /// 策略可在运行期被任一壳修改（Web 改设置、CLI 加 `--read-only`），故加锁。
    policy: RwLock<SafetyPolicy>,
    /// 空闲清理线程是否已经起来了（按需启动，见 `ensure_sweeper`）
    sweeper_started: std::sync::atomic::AtomicBool,
}

impl DbMindEngine {
    /// 打开指定元数据库。
    pub fn open(store_path: &Path) -> Result<Self> {
        Self::from_store(Store::open(store_path)?)
    }

    /// 打开默认元数据库（`~/.dbmind/dbmind.db`）。
    pub fn open_default() -> Result<Self> {
        Self::open(&paths::default_store_path())
    }

    /// 内存态引擎（测试与临时会话）。
    pub fn in_memory() -> Result<Self> {
        Self::from_store(Store::open_in_memory()?)
    }

    pub fn from_store(store: Store) -> Result<Self> {
        let store = Arc::new(store);
        let policy = SafetyPolicy::new()
            .with_production_protection(store.get_bool_setting(Store::KEY_PROTECT_PRODUCTION, false)?)
            .with_ai_write(store.get_bool_setting(Store::KEY_AI_WRITE_ENABLED, false)?)
            .with_block_dangerous(store.get_bool_setting(Store::KEY_BLOCK_DANGEROUS, false)?)
            .with_max_write_rows(store.get_usize_setting(Store::KEY_MAX_WRITE_ROWS, 0)? as u64);
        let engine = Self {
            store,
            drivers: DriverRegistry::with_builtin(),
            cancels: CancelRegistry::new(),
            policy: RwLock::new(policy),
            sweeper_started: std::sync::atomic::AtomicBool::new(false),
        };
        // 会话相关设置：开机就按设置生效（驱动持有共享原子量，实时读到）
        engine.apply_session_limits()?;
        // 旧版 TLS 开关同步给宿主 spawn 层（spawn 读静态原子量；设置变更时也会同步）
        let legacy_tls = engine
            .store
            .get_setting(Store::KEY_ALLOW_LEGACY_TLS)?
            .as_deref()
            == Some("true");
        crate::agent::set_legacy_tls_enabled(legacy_tls);
        // 隧道空闲回收时长 + 历史保留策略：开机先对齐一遍（旧库可能积了几万行历史）
        engine.apply_tunnel_idle_timeout();
        engine.prune_history();
        engine.reconcile_shadow_read_only();
        engine.refresh_agent_hosts();
        // 首次运行初始化：目录骨架 + （只在真正全新时）示例库与示例连接。
        // 失败只记警告 —— 初始化没做好不该拦住用户开库。
        match crate::bootstrap::ensure_first_run(engine.store()) {
            Ok(report) if !report.is_empty() => tracing::info!(
                dirs = report.created_dirs.len(),
                "数据目录初始化完成"
            ),
            Ok(_) => {}
            Err(err) => {
                tracing::warn!(target: "dbmind::bootstrap", error = %err, "数据目录初始化失败")
            }
        }
        Ok(engine)
    }

    /// 把「连接用到的主机」导出成 Java 宿主的自有 hosts 文件
    /// （生成规则与实测数据见 [`crate::agent::prepare_agent_hosts`]）。
    ///
    /// 为什么由引擎来做：宿主是**独立 JVM**，它自己不知道这台机器配了哪些连接；
    /// 而 MySQL 驱动建连接时会反解服务端 IP，企业 DNS 没有 PTR 记录 ⇒
    /// 每新建一条会话白等约 4.6 秒。引擎恰好是唯一能看到全部连接的地方。
    ///
    /// 失败只留一行日志：拿不到这份文件时宿主照旧走系统解析，与优化前完全一致。
    fn refresh_agent_hosts(&self) {
        let hosts: Vec<String> = match self.store.list_connections() {
            Ok(records) => records
                .into_iter()
                .filter_map(|record| record.config.host.clone())
                .collect(),
            Err(err) => {
                eprintln!("[dbmind] 读取连接列表失败，宿主 hosts 未刷新: {err}");
                return;
            }
        };
        crate::agent::prepare_agent_hosts(&hosts);
    }

    /// 把会话相关设置推给驱动与预算。开机与设置变更时都要走一遍。
    fn apply_session_limits(&self) -> Result<()> {
        let budget = self.drivers.session_budget();
        budget.set_cap(
            self.store
                .get_usize_setting(Store::KEY_SESSION_MAX_PER_HOST, DEFAULT_SESSION_MAX_PER_HOST)?,
        );
        let idle = self.store.get_usize_setting(Store::KEY_SESSION_IDLE_TIMEOUT, 0)?;
        budget.set_idle_timeout_secs(idle);
        if idle > 0 {
            self.ensure_sweeper();
        }
        Ok(())
    }

    /// 启动空闲会话清理线程（幂等）。
    ///
    /// 为什么**按需**启动：默认（不按空闲回收）时不该有任何后台线程；而用户在设置里
    /// 打开它之后，线程必须立刻起来 —— 不能等重启（那会让「我改了设置却没反应」）。
    /// 为什么必须是线程：这个功能要解决的正是「用户离开了、连接却一直占着」，
    /// 「下次操作时顺手扫一下」在这件事上等于没做。
    /// 线程只持有 `Weak`：引擎被 drop 后，它下一轮醒来就自己退出。
    fn ensure_sweeper(&self) {
        use std::sync::atomic::Ordering;
        if self.sweeper_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(&self.drivers.session_budget());
        let started = std::thread::Builder::new()
            .name("dbmind-session-sweeper".to_string())
            .spawn(move || loop {
                std::thread::sleep(SESSION_SWEEP_INTERVAL);
                let Some(budget) = weak.upgrade() else { return };
                let freed = budget.sweep_idle();
                if freed > 0 {
                    tracing::debug!(target: "dbmind::agent", freed, "按空闲时长回收了会话");
                }
            });
        if let Err(err) = started {
            // 起不来不致命：少了主动归还而已，配额与「满时回收」都还在
            self.sweeper_started.store(false, Ordering::SeqCst);
            tracing::warn!(target: "dbmind::engine", error = %err, "会话清理线程未能启动");
        }
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// 供测试断言「会话设置到达了预算」（只存在于测试构建）。
    #[cfg(test)]
    pub(crate) fn session_budget(&self) -> Arc<crate::drivers::SessionBudget> {
        self.drivers.session_budget()
    }

    pub fn drivers(&self) -> &DriverRegistry {
        &self.drivers
    }

    /// 取策略快照（避免调用期间被其它线程改动）。
    pub fn policy(&self) -> SafetyPolicy {
        self.policy.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn update_policy(&self, mutate: impl FnOnce(&mut SafetyPolicy)) {
        let mut guard = self.policy.write().unwrap_or_else(|e| e.into_inner());
        mutate(&mut guard);
    }

    /// 类型目录：来自 YAML 的编译期生成物 + 运行期实现状态。
    pub fn types(&self) -> Vec<TypeDescriptor> {
        all_descriptors()
    }

    pub fn resolve_kind(&self, key: &str) -> Result<ConnectionKind> {
        ConnectionKind::from_key(key)
            .ok_or_else(|| DbMindError::new(ErrorCode::ConnTypeUnknown, format!("未知连接类型「{key}」")))
    }

    // ------------------------------------------------------------ 连接管理

    pub fn list_connections(&self) -> Result<Vec<ConnectionRecord>> {
        self.store.list_connections()
    }

    pub fn find_connection(&self, id_or_name: &str) -> Result<Option<ConnectionRecord>> {
        self.store.find_connection(id_or_name)
    }

    pub fn require_connection(&self, id_or_name: &str) -> Result<ConnectionRecord> {
        self.store.require_connection(id_or_name)
    }

    pub fn add_connection(&self, config: ConnectionConfig) -> Result<ConnectionRecord> {
        let record = self.store.insert_connection(&config)?;
        // 影子连接出生就继承主连接的只读标记（见 `inherit_read_only_from_base`）
        let record = self.inherit_read_only_from_base(&record)?.unwrap_or(record);
        // 新连接的主机要进宿主 hosts —— 否则它第一次反解仍要等约 4.6 秒
        self.refresh_agent_hosts();
        Ok(record)
    }

    /// 影子连接（跨库浏览按需生成的记录）出生时**继承主连接的只读标记**。
    ///
    /// 不继承的后果实测过：主连接上 `UPDATE` 被闸门拦下，可同一个库节点上（走影子）执行
    /// **同样的语句却能写进去** —— 用户看到的就是「数据源开了只读，还是能执行 insert」。
    ///
    /// 收口在这里（而不是让每个生成点自己记得带上标记）的理由：影子是**内核之外**按需造出来的，
    /// 造它的地方将来还会增加，而「只读」是安全属性 —— 漏一处的代价是闸门被绕过。
    /// 开机时把「主连接只读、影子却没跟上」的记录补齐。
    ///
    /// 为什么要有这一趟：影子的只读标记是**跟着主连接走**的（见 [`Self::set_read_only`]），
    /// 而这个不变量是后来才补上的 —— 之前创建的影子会一直停在 `read_only = false`，
    /// 于是老库里「开了只读的数据源」在跨库时依旧能写。只靠「下次切换开关时顺带修正」
    /// 等于要求用户先关掉再打开一次，而他要的只是「现在就该拦住」。
    ///
    /// 失败只留一行日志：这一步是**修数据**，修不成不该拦住开库（闸门本身仍然有效）。
    fn reconcile_shadow_read_only(&self) {
        let records = match self.store.list_connections() {
            Ok(records) => records,
            Err(err) => {
                tracing::warn!(target: "dbmind::engine", error = %err.message, "读取连接清单失败，跳过影子只读校准");
                return;
            }
        };
        let mut fixed = 0usize;
        for shadow in records.iter().filter(|item| item.is_shadow() && !item.read_only) {
            let base_is_read_only = shadow
                .shadow_of()
                .and_then(|base_id| self.store.require_connection(base_id).ok())
                .map(|base| base.read_only)
                .unwrap_or(false);
            if !base_is_read_only {
                continue;
            }
            if let Err(err) = self.store.set_read_only(&shadow.id, true) {
                tracing::warn!(target: "dbmind::engine", connection = %shadow.name(), error = %err.message, "影子只读校准失败");
                continue;
            }
            let id = shadow.id.clone();
            self.update_policy(|policy| policy.mark_read_only(id));
            fixed += 1;
        }
        if fixed > 0 {
            tracing::info!(target: "dbmind::engine", fixed, "已把影子连接的只读标记与主连接对齐");
        }
    }

    fn inherit_read_only_from_base(&self, record: &ConnectionRecord) -> Result<Option<ConnectionRecord>> {
        let Some(base_id) = record.shadow_of() else {
            return Ok(None);
        };
        let Ok(base) = self.store.require_connection(base_id) else {
            return Ok(None);
        };
        if !base.read_only {
            return Ok(None);
        }
        let updated = self.store.set_read_only(&record.id, true)?;
        let id = updated.id.clone();
        self.update_policy(|policy| policy.mark_read_only(id));
        Ok(Some(updated))
    }

    /// 更新连接配置。
    ///
    /// **口令与 extra 采用「缺省即保留」语义**。原因是界面上的必然情形：
    /// 详情接口只写不读，编辑连接时前端**拿不到明文口令**，交回来的配置里
    /// `password` 自然是 `None`；若这里原样写库，就会把已存口令清成 NULL ——
    /// 用户只是改个库名或端口，连接就静默失效，而那之后报的是「认证失败」，
    /// 排查方向会被带偏很远（这条是接前端时实测撞到的）。
    /// 想**真正清掉**口令：给空串（`Some("")`）。
    pub fn update_connection(&self, id: &str, mut config: ConnectionConfig) -> Result<ConnectionRecord> {
        let existing = self.store.require_connection(id)?;
        if config.password.is_none() {
            config.password = existing.config.password.clone();
        }
        // extra 同理：它装着连接级并发上限等前向兼容配置，界面不会回传
        if config.extra.is_none() {
            config.extra = existing.config.extra.clone();
        }
        let record = self.store.update_connection(id, &config)?;
        // 配置可能改了目标库/主机 ⇒ 旧结构缓存描述的是**另一个库**，必须作废
        let _ = self.store.invalidate_schema(&record.id, None);
        // 主机可能改了：不刷新的话，新主机第一次反解仍要白等约 4.6 秒
        self.refresh_agent_hosts();
        Ok(record)
    }

    pub fn remove_connection(&self, id_or_name: &str) -> Result<bool> {
        let removed = self.store.delete_connection(id_or_name)?;
        self.refresh_agent_hosts();
        Ok(removed)
    }

    /// 断开某连接的缓存会话（**不动连接记录本身**）。
    ///
    /// 用途：数据库侧改了权限/密码后，内核连接池里的旧会话还带着旧的全局权限快照
    ///（MySQL 的全局权限变更只对新建连接生效），用户不必重启应用 ——
    /// 界面「关闭连接」时调用，重开树节点即为全新会话。
    /// 会话按类型池化，同类型的其它连接只是丢了缓存会话，下次用时重建（几百毫秒）。
    pub fn disconnect_sessions(&self, id_or_name: &str) -> Result<usize> {
        let record = self.store.find_connection(id_or_name)?;
        let Some(record) = record else { return Ok(0) };
        // 「关闭连接」要**真正断**：只断这一个连接的会话（不再按类型 disconnect_all
        // 误伤同类其它连接），并且**影子连接跟着断** —— 跨库浏览按需生成的那些影子
        // 各自有独立泳道/会话，留着它们，树上的库节点下次展开还是旧会话。
        let kind = record.kind();
        let mut n = self.drivers.disconnect_connection(kind, record.config.name.as_str());
        for shadow in self.store.list_connections()? {
            if shadow.shadow_of() == Some(record.id.as_str()) {
                n += self.drivers.disconnect_connection(kind, shadow.config.name.as_str());
            }
        }
        Ok(n)
    }

    /// 只断**某个数据库**的会话（界面上「关闭数据库」用）。
    ///
    /// 该库的语句跑在它的**影子连接**上（跨库浏览按需生成，database 指向这个库），
    /// 所以按「shadow_of = 该连接 且 database = 该库」找到影子，断它的会话。
    /// 主连接的共享元数据会话不受影响 —— 连接本身还开着。
    pub fn disconnect_database(&self, id_or_name: &str, database: &str) -> Result<usize> {
        let record = self.store.find_connection(id_or_name)?;
        let Some(record) = record else { return Ok(0) };
        let kind = record.kind();
        let mut n = 0usize;
        for shadow in self.store.list_connections()? {
            if shadow.shadow_of() == Some(record.id.as_str())
                && shadow.config.database.as_deref() == Some(database)
            {
                n += self.drivers.disconnect_connection(kind, shadow.config.name.as_str());
            }
        }
        Ok(n)
    }

    /// 切换只读标记：同时更新存储与内存策略，避免「库里改了、策略没改」的漂移。
    ///
    /// **影子连接跟着一起改**（跨库浏览按需生成的那些记录）：用户看到的开关只在主连接上，
    /// 而语句是在影子身上执行的。不跟着改的话，「连接设为只读」在点开任意一个库之后就被绕过
    /// —— 实测：主连接上 `UPDATE` 被拦，从库节点执行同样的语句却能写进去。
    /// 关掉开关时同样要清，否则只读一旦打开就再也关不掉（库里那条影子还留着 true）。
    pub fn set_read_only(&self, id_or_name: &str, read_only: bool) -> Result<ConnectionRecord> {
        let record = self.store.set_read_only(id_or_name, read_only)?;
        let mut ids = vec![record.id.clone()];
        for shadow in self.store.list_connections()? {
            if shadow.shadow_of() == Some(record.id.as_str()) {
                self.store.set_read_only(&shadow.id, read_only)?;
                ids.push(shadow.id);
            }
        }
        self.update_policy(|policy| {
            for id in &ids {
                if read_only {
                    policy.mark_read_only(id.clone());
                } else {
                    policy.unmark_read_only(id);
                }
            }
        });
        Ok(record)
    }

    // ------------------------------------------------------------ 自检与结构

    pub fn test_connection(&self, id_or_name: &str) -> Result<ConnectReport> {
        let record = self.store.require_connection(id_or_name)?;
        let driver = self.drivers.resolve(record.kind())?;
        let read_only = self.policy().is_read_only(&record);
        let mut report = driver.test(&record.config, read_only).map_err(|e| {
            // 自检阶段的失败统一归到「连接失败」，前端据此走「检查配置」引导
            if e.code == ErrorCode::ConnInvalid {
                e
            } else {
                DbMindError::new(
                    ErrorCode::ConnConnectFailed,
                    format!("连接「{}」自检失败", record.name()),
                )
                .with_detail(e.to_string())
            }
        })?;
        report.connection_id = record.id;
        Ok(report)
    }

    /// 用**一份配置**做连通性自检，**不落地到连接库**。
    ///
    /// 与 [`test_connection`](Self::test_connection) 的区别只有一处：那个要求「已存连接」
    /// （按 id 取记录、用库里存的口令），这个直接用调用方给的配置去连。
    ///
    /// 存在的意义是**消除持久化副作用**：界面上「测试连接」测的是**还没保存的表单**，
    /// 早前只能在库里建一条临时连接、测完再删 —— 删除一旦失败（进程被杀、请求中断、
    /// 驱动占用），那条连接就永久留在用户的连接列表里，表现为对象浏览器里凭空多出
    /// 几行 `名字 (probe exec_…)` 的连接，而用户从没建过它们。
    ///
    /// `read_only` 由调用方传入：这里没有连接记录，策略层（按环境强制只读等）无从判断，
    /// 而连通性自检本身也不该受只读标记影响。
    pub fn test_config(&self, config: &ConnectionConfig, read_only: bool) -> Result<ConnectReport> {
        let driver = self.drivers.resolve(config.kind)?;
        let mut report = driver.test(config, read_only).map_err(|e| {
            // 与 `test_connection` 同样的归类：配置本身不合法就原样报，
            // 其余一律归到「连接失败」，前端据此走「检查配置」引导
            if e.code == ErrorCode::ConnInvalid {
                e
            } else {
                DbMindError::new(
                    ErrorCode::ConnConnectFailed,
                    format!("连接「{}」自检失败", config.name),
                )
                .with_detail(e.to_string())
            }
        })?;
        // 没有记录可言：明确留空，避免调用方以为测的是某个已存连接
        report.connection_id = String::new();
        Ok(report)
    }

    /// 结构缓存的存活时间（**默认值**，实际以设置 `schema.ttlSecs` 为准）。
    ///
    /// 5 分钟是折中：结构变动很少，但「同事刚加的表」也不该等到重启才看见。
    /// 需要立刻看到时用 `--refresh`（CLI）/ `?refresh=true`（Web）/ MCP 默认走新鲜读，
    /// 或者把设置里的 TTL 调小、点「立即刷新」。
    const DEFAULT_SCHEMA_TTL_SECS: i64 = 300;

    /// 结构缓存有效期：读设置 `schema.ttlSecs`（30..=86400 秒）。
    ///
    /// 为什么从写死改成可配：改了表结构想立刻在树上看到，只有两条路 —— 记得每次
    /// 带 `?refresh=true`，或干等 5 分钟。前者靠记性，后者靠耐心；把「等多久」交给
    /// 用户，配合设置页的「刷新结构缓存」按钮，两条路都通。
    fn schema_ttl_secs(&self) -> i64 {
        self.store
            .get_setting(Store::KEY_SCHEMA_TTL)
            .ok()
            .flatten()
            .and_then(|value| value.trim().parse::<i64>().ok())
            .map(|secs| secs.clamp(30, 86_400))
            .unwrap_or(Self::DEFAULT_SCHEMA_TTL_SECS)
    }

    /// 对象清单（走缓存）。
    pub fn list_tables(&self, id_or_name: &str) -> Result<Vec<TableInfo>> {
        self.objects(id_or_name, false)
    }

    /// 对象清单（绕过缓存）。
    pub fn list_tables_fresh(&self, id_or_name: &str) -> Result<Vec<TableInfo>> {
        self.objects(id_or_name, true)
    }

    fn objects(&self, id_or_name: &str, refresh: bool) -> Result<Vec<TableInfo>> {
        let record = self.store.require_connection(id_or_name)?;
        let key = Store::SCHEMA_OBJECTS_KEY;
        if !refresh {
            if let Some(payload) = self.store.fresh_schema(&record.id, key, self.schema_ttl_secs())? {
                match serde_json::from_str(&payload) {
                    Ok(tables) => return Ok(tables),
                    Err(e) => {
                        // 缓存内容坏了就当没命中并清掉：宁可多查一次，
                        // 也不要因为一条脏缓存让结构页永远打不开
                        tracing::warn!(target: "dbmind::engine", error = %e, "结构缓存内容损坏，已丢弃");
                        self.store.invalidate_schema(&record.id, Some(key))?;
                    }
                }
            }
        }
        let read_only = self.policy().is_read_only(&record);
        let tables = self
            .drivers
            .resolve(record.kind())?
            .list_tables(crate::drivers::MetaCall {
                config: &record.config,
                read_only,
            })?;
        self.store
            .put_schema(&record.id, key, &serde_json::to_string(&tables)?)?;
        Ok(tables)
    }

    /// 单表列（走缓存）。
    pub fn list_columns(&self, id_or_name: &str, table: &str) -> Result<Vec<ColumnDetail>> {
        self.columns(id_or_name, table, false)
    }

    /// 单表列（绕过缓存）。
    pub fn list_columns_fresh(&self, id_or_name: &str, table: &str) -> Result<Vec<ColumnDetail>> {
        self.columns(id_or_name, table, true)
    }

    fn columns(&self, id_or_name: &str, table: &str, refresh: bool) -> Result<Vec<ColumnDetail>> {
        let record = self.store.require_connection(id_or_name)?;
        let key = Store::schema_columns_key(table);
        if !refresh {
            if let Some(payload) = self.store.fresh_schema(&record.id, &key, self.schema_ttl_secs())? {
                if let Ok(columns) = serde_json::from_str(&payload) {
                    return Ok(columns);
                }
                self.store.invalidate_schema(&record.id, Some(&key))?;
            }
        }
        let read_only = self.policy().is_read_only(&record);
        let columns = self.drivers.resolve(record.kind())?.list_columns(
            crate::drivers::MetaCall {
                config: &record.config,
                read_only,
            },
            table,
        )?;
        self.store
            .put_schema(&record.id, &key, &serde_json::to_string(&columns)?)?;
        Ok(columns)
    }

    /// 结构缓存概览（界面显示「缓存于 N 秒前」）。
    pub fn schema_cache_info(&self, id_or_name: &str) -> Result<crate::types::SchemaCacheInfo> {
        let record = self.store.require_connection(id_or_name)?;
        self.store.schema_cache_info(&record.id)
    }

    /// 手工作废某连接的结构缓存。
    pub fn clear_schema_cache(&self, id_or_name: &str) -> Result<usize> {
        let record = self.store.require_connection(id_or_name)?;
        self.store.invalidate_schema(&record.id, None)
    }

    /// 作废**所有**连接的结构缓存（设置页「刷新结构缓存」的入口）。
    ///
    /// 按连接逐个清在 web 层也能拼出来，但那要求 web 层知道「有哪些连接」、
    /// 还得记得把影子连接算进去 —— 收口在内核，语义与 [`Self::clear_schema_cache`] 一致。
    pub fn clear_all_schema_cache(&self) -> Result<usize> {
        let mut cleared = 0usize;
        for record in self.store.list_connections()? {
            cleared += self.store.invalidate_schema(&record.id, None)?;
        }
        Ok(cleared)
    }

    /// 执行成功后按需作废结构缓存。
    ///
    /// 两条规则，都是「谁可能改结构」：
    /// - **结构变更**：一律作废整个连接（列也可能跟着变）；
    /// - **数据写入**：只有**非 SQL** 协议才可能顺带造出对象 ——
    ///   Mongo 的 `insertOne` 会建集合、ES 的 `POST /idx/_doc` 会建索引，
    ///   而 SQL 的 `insert` 不会。搞混的代价是双向的：漏了会看到「不存在的表」，
    ///   多了则每次都白查一遍元数据。
    fn invalidate_schema_after(&self, connection: &ConnectionRecord, kind: crate::StatementKind) {
        let key = match kind {
            crate::StatementKind::Ddl => None,
            crate::StatementKind::Write if connection.kind().protocol() != crate::RuntimeProtocol::Sql => {
                Some(Store::SCHEMA_OBJECTS_KEY)
            }
            _ => return,
        };
        if let Err(e) = self.store.invalidate_schema(&connection.id, key) {
            // 作废失败不影响查询结果：最坏情况是用户多看 5 分钟前的结构，且有日志可查
            tracing::warn!(target: "dbmind::engine", error = %e, "作废结构缓存失败");
        }
    }

    // ------------------------------------------------------------ 执行

    /// 统一的执行入口：**策略闸门 → 驱动执行 → 历史留痕**。
    ///
    /// 所有**早退失败**（连接不存在、被策略拦截、驱动未接入）同样留痕：
    /// 被拒的写操作恰恰是审计里最该看到的东西，不能因为「没执行」就不记。
    pub fn execute(&self, request: QueryRequest, ctx: AccessContext) -> Result<QueryResult> {
        let started = Instant::now();
      let options = request.options.clamp();

  // 程序自己发的调用（结构浏览 / 表格预览分页 / 元数据探测 / 界面功能驱动的语句）
  // **不进历史**：历史是"用户执行过什么"的账本，它们混进来会把真实操作淹掉
  // （首页「最近查询」就是受害者）。
  // 判据是**显式标记**，不是去猜 SQL 文本长什么样。两种标记都认：
  //   - `request.internal`：与连接选择无关的干净开关（多数调用方用这个）；
  //   - 会话键的 `internal:` 前缀：早期写法，会让这类语句落到单独一条物理会话上，仍在用。
  let record_history = !request.internal
      && !request
          .session
          .as_deref()
          .map(|s| s.starts_with(INTERNAL_SESSION_PREFIX))
          .unwrap_or(false);

        // 会话亲和键的长度上限（见函数的文档里的说明，常量定义在文件顶部）
        const MAX_SESSION_KEY: usize = 64;

/// 内部会话前缀：程序自己发的调用（结构浏览 / 预览分页 / 探测 / 同步）都用它。
/// 内核据此**不写历史** —— 历史是"用户执行过什么"的账本，内部请求混进来会把真实操作淹掉。
pub const INTERNAL_SESSION_PREFIX: &str = "internal:";

        // 1) 连接
        let mut connection = match self.store.require_connection(&request.connection) {
            Ok(connection) => connection,
            Err(err) => {
                let entry = NewHistoryEntry {
                    connection_id: None,
                    connection_name: Some(request.connection.clone()),
                    sql: request.sql.clone(),
                    status: HistoryStatus::Error,
                    row_count: 0,
                    duration_ms: started.elapsed().as_millis() as u64,
                    error_code: Some(err.code_str().to_string()),
                };
                if record_history { self.record_entry(&entry); }
                return Err(err);
            }
        };

        // 会话亲和键：界面每个页面给一个 ⇒ 每个标签页一条数据库会话。
        // 校验放在这里（连接已解析出来），失败照样留痕 —— 与其它早退一致。
        let session = request
            .session
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());
        if let Some(key) = session {
            if key.chars().count() > MAX_SESSION_KEY {
                let err = DbMindError::new(
                    ErrorCode::QueryInvalid,
                    format!("会话键过长（最多 {MAX_SESSION_KEY} 字符）"),
                );
                self.record(record_history, 
                    &connection,
                    &request.sql,
                    HistoryStatus::Error,
                    0,
                    started.elapsed().as_millis() as u64,
                    Some(err.code_str().to_string()),
                );
                return Err(err);
            }
        }

        // 影子连接的**安全属性跟随主连接**（闸门前同步，主连接是唯一真相源）：
        // 影子是跨库浏览时按需生成的快照，主连接后来改了安全属性，影子还揣着
        // 创建那一刻的旧值 ——
        // - 环境标注：主连接补标「生产」，编辑器切到影子库照样 INSERT 成功（真机踩过）；
        // - 只读标记：出生时继承的快照同样会过期。
        // 只动 env 角标 / 分组两组键与 read_only，`shadowFor` 等影子标记原样保留。
        if let Some(parent_id) = connection.shadow_of() {
            if let Ok(parent) = self.store.require_connection(parent_id) {
                let mut extra = match connection.config.extra.as_ref() {
                    Some(serde_json::Value::Object(map)) => map.clone(),
                    _ => serde_json::Map::new(),
                };
                for key in [
                    crate::types::EXTRA_ENV_BADGE,
                    crate::types::EXTRA_GROUP,
                    crate::types::EXTRA_GROUP_LEGACY,
                ] {
                    match parent.config.extra.as_ref().and_then(|e| e.get(key)) {
                        Some(v) => {
                            extra.insert(key.to_string(), v.clone());
                        }
                        None => {
                            extra.remove(key);
                        }
                    }
                }
                connection.config.extra = Some(serde_json::Value::Object(extra));
                connection.read_only = parent.read_only;
            }
        }

        // 2) 闸门：读写在执行前定死，壳层无法绕过。
        // 内部链路（数据传输/对比/导入导出的批量执行）放行多语句批 —— identity 的
        // SET+INSERT+SET、删库的 ALTER+DROP 都必须同批才能生效（会话级开关/占用清理）
        let policy = self.policy();
        if let Err(err) = policy.check(&connection, &request.sql, ctx, request.internal) {
            self.record(record_history, 
                &connection,
                &request.sql,
                HistoryStatus::Error,
                0,
                started.elapsed().as_millis() as u64,
                Some(err.code_str().to_string()),
            );
            return Err(err);
        }

        // 2.5) 写操作影响行数预估：`maxWriteRows` 开着时，UPDATE/DELETE 在真正执行前
        //      先 `COUNT(*)` 一把，预估超过上限直接拒绝 —— 等执行完再发现「影响 20 万行」
        //      就晚了。预估走内部调用（不进历史；读语句不触发本检查，不会递归）。
        //      预估失败（解析不了复合形态 / COUNT 执行报错）⇒ 跳过检查放行原语句：
        //      预估是尽力而为的保险丝，不能反过来卡死正常操作。
        if policy.max_write_rows > 0 {
            if let Some(target) = crate::statement::write_target(
                connection.kind().protocol(),
                &request.sql,
            ) {
                let count_sql = match &target.where_clause {
                    Some(w) => format!("select count(*) from {} where {}", target.table, w),
                    None => format!("select count(*) from {}", target.table),
                };
                let mut probe = QueryRequest::new(connection.id.as_str(), count_sql);
                probe.internal = true;
                if let Ok(result) = self.execute(probe, ctx) {
                    if let Some(affected) = result
                        .rows
                        .first()
                        .and_then(|row| row.first())
                        .and_then(|cell| match cell {
                            CellValue::Integer(v) => Some(*v as u64),
                            CellValue::Real(v) => Some(*v as u64),
                            CellValue::Text(t) => t.trim().parse::<u64>().ok(),
                            _ => None,
                        })
                    {
                        if affected > policy.max_write_rows {
                            let err = DbMindError::new(
                                ErrorCode::SafetyRowLimit,
                                format!(
                                    "这次操作预计影响 {} 行数据，超过了「单次改动行数上限」（{}）。请缩小范围后再试，或到设置 → 安全与会话中调整上限",
                                    affected, policy.max_write_rows
                                ),
                            );
                            self.record(record_history,
                                &connection,
                                &request.sql,
                                HistoryStatus::Error,
                                0,
                                started.elapsed().as_millis() as u64,
                                Some(err.code_str().to_string()),
                            );
                            return Err(err);
                        }
                    }
                }
            }
        }

        // 3) 驱动
        let driver = match self.drivers.resolve(connection.kind()) {
            Ok(driver) => driver,
            Err(err) => {
                self.record(record_history, 
                    &connection,
                    &request.sql,
                    HistoryStatus::Error,
                    0,
                    started.elapsed().as_millis() as u64,
                    Some(err.code_str().to_string()),
                );
                return Err(err);
            }
        };
        let execution_id = request
            .execution_id
            .clone()
            .unwrap_or_else(crate::new_execution_id);
        // 显式指定的只读优先：内部浏览类语句（全是只读 SELECT）用它来**复用结构浏览那条
        // 只读会话**，否则同一库同一件事会各自建一条物理连接（实测一次 /tables 两条连接）。
        // 缺省仍按连接策略算 —— 用户自己的查询行为完全不变。
        let read_only = request
            .read_only
            .unwrap_or_else(|| policy.is_read_only(&connection));
        let cancel = self.cancels.register(&execution_id);

        let outcome = driver.query(QueryCall {
            config: &connection.config,
            sql: &request.sql,
            options: &options,
            execution_id: &execution_id,
            cancel: &cancel,
            read_only,
            session,
        });

        // 4) 无论成败都要注销登记，并留痕
        self.cancels.finish(&execution_id);
        let duration_ms = started.elapsed().as_millis() as u64;

        match outcome {
            Ok(mut result) => {
                result.execution_id = execution_id;
                result.connection_id = connection.id.clone();
                result.connection_name = connection.name().to_string();
                result.duration_ms = duration_ms;
                // 「结果来自哪个对象」由内核统一判定：只有读语句的结果才谈得上就地编辑，
                // 写语句即使有对象名也没有可改的行
                if result.statement_kind == crate::StatementKind::Read {
                    result.source_object =
                        crate::statement::source_object(connection.kind().protocol(), &request.sql);
                    // 行标识：宿主已经给了就不覆盖（Mongo 的 `_id` 类型只有它知道），
                    // 否则按协议事实推（只有 ES 推得出来，见 statement::derived_row_identity）
                    if result.row_identity.is_none() {
                        result.row_identity = crate::statement::derived_row_identity(
                            connection.kind().protocol(),
                            &result.columns,
                        );
                    }
                }
                if result.truncated {
                    result.notices.push(format!(
                        "结果超过 {} 行已截断，建议加 LIMIT 或导出",
                        options.max_rows
                    ));
                }
                self.record(record_history, 
                    &connection,
                    &request.sql,
                    HistoryStatus::Ok,
                    result.row_count,
                    duration_ms,
                    None,
                );
                // 结构相关的执行成功后才作废缓存：失败不该让缓存失效（白查一遍）
                self.invalidate_schema_after(&connection, result.statement_kind);
                Ok(result)
            }
            Err(err) => {
                let status = match err.code {
                    ErrorCode::QueryCanceled => HistoryStatus::Canceled,
                    _ => HistoryStatus::Error,
                };
                self.record(record_history, 
                    &connection,
                    &request.sql,
                    status,
                    0,
                    duration_ms,
                    Some(err.code_str().to_string()),
                );
                Err(err)
            }
        }
    }

    /// 请求取消某个执行；返回是否命中。
    pub fn cancel(&self, execution_id: &str) -> bool {
        self.cancels.cancel(execution_id)
    }

    pub fn active_executions(&self) -> Vec<String> {
        self.cancels.active()
    }

    /// 生成一个执行 id（供壳层在发起前先拿到，用于取消与前端关联）。
    pub fn next_execution_id() -> String {
        crate::new_execution_id()
    }

    /// 会话级事务控制（事务模式）：`begin` 关掉该会话的 autocommit，
    /// `commit` / `rollback` 提交或回滚并交回 autocommit。
    ///
    /// 会话键由 web 层传「与 SQL 编辑器相同的亲和键」⇒ 事务精确落在编辑器
    /// 那条物理连接上（全 agent 数据源通用，见 [`crate::drivers::Driver::tx_control`]）。
    /// 动作**无条件留痕**：BEGIN/COMMIT/ROLLBACK 是审计里最该看到的东西。
    pub fn tx_control(
        &self,
        connection_id: &str,
        session: &str,
        action: crate::drivers::TxAction,
    ) -> Result<serde_json::Value> {
        let started = Instant::now();
        // 策略闸门：只读连接不允许开手动事务（开 autocommit=false 就是为了写）
        let connection = self.store.require_connection(connection_id)?;
        let read_only = self.policy().is_read_only(&connection);
        if read_only {
            let err = DbMindError::new(
                ErrorCode::SafetyReadOnly,
                "这条连接是只读的，不能开启事务模式",
            );
            self.record(
                true,
                &connection,
                action.as_sql(),
                HistoryStatus::Error,
                0,
                started.elapsed().as_millis() as u64,
                Some(err.code_str().to_string()),
            );
            return Err(err);
        }
        let driver = self.drivers.resolve(connection.kind())?;
        let outcome = driver.tx_control(&connection.config, read_only, session, action);
        let duration_ms = started.elapsed().as_millis() as u64;
        match &outcome {
            Ok(_) => self.record(
                true,
                &connection,
                action.as_sql(),
                HistoryStatus::Ok,
                0,
                duration_ms,
                None,
            ),
            Err(err) => self.record(
                true,
                &connection,
                action.as_sql(),
                HistoryStatus::Error,
                0,
                duration_ms,
                Some(err.code_str().to_string()),
            ),
        }
        outcome
    }

    fn record(
        &self,
        record_history: bool,
        connection: &ConnectionRecord,
        sql: &str,
        status: HistoryStatus,
        row_count: usize,
        duration_ms: u64,
        error_code: Option<String>,
    ) {
        let entry = NewHistoryEntry {
            connection_id: Some(connection.id.clone()),
            connection_name: Some(connection.name().to_string()),
            sql: sql.to_string(),
            status,
            row_count,
            duration_ms,
            error_code,
        };
        if record_history { self.record_entry(&entry); }
    }

    fn record_entry(&self, entry: &NewHistoryEntry) {
        if let Err(e) = self.store.record_history(entry) {
            // 留痕失败不能影响主流程
            tracing::warn!(target: "dbmind::engine", error = %e, "写入历史失败");
            return;
        }
        // 顺手清一次：一条 INSERT 换一条 DELETE，比「积了十万行再大扫除」平稳得多
        self.prune_history();
    }

    /// 按设置清理查询历史（`history.maxEntries` / `history.retentionDays`）。
    /// 清理失败只记日志：这是修数据，不该让主流程为它失败。
    pub fn prune_history(&self) -> usize {
        let max = self
            .store
            .get_usize_setting(Store::KEY_HISTORY_MAX_ENTRIES, 1000)
            .unwrap_or(1000)
            .min(100_000);
        let days = self
            .store
            .get_usize_setting(Store::KEY_HISTORY_RETENTION_DAYS, 30)
            .unwrap_or(30)
            .min(3650) as u32;
        match self.store.prune_history(max, days) {
            Ok(0) => 0,
            Ok(n) => {
                tracing::debug!(target: "dbmind::engine", removed = n, max_entries = max, retention_days = days, "已按保留策略清理查询历史");
                n
            }
            Err(e) => {
                tracing::warn!(target: "dbmind::engine", error = %e, "清理查询历史失败");
                0
            }
        }
    }

    /// 元数据库一致性快照（数据目录迁移用），见 [`crate::storage::Store::backup_into`]。
    pub fn backup_store_into(&self, path: &Path) -> Result<()> {
        self.store.backup_into(path)
    }

    /// 设置页「缓存」页签的体量清单：`(键, 行数)`。**白名单**——键名与表名的映射
    /// 收死在这里，web 层只透传，永远不要从请求里拼 SQL 表名。
    /// 不含术语表 / 采纳示例 / 质量规则：那是用户逐条维护的知识数据，不该出现在
    /// 「勾选清理」的清单里。
    pub fn cache_report(&self) -> Result<Vec<(&'static str, u64)>> {
        const TABLES: &[(&str, &str)] = &[
            ("schemaCache", "schema_cache"),
            ("queryHistory", "query_history"),
            ("aiAudit", "ai_audit"),
            ("aiUsageDays", "ai_usage_days"),
            ("aiUsageModels", "ai_usage_models"),
            ("kbDocs", "kb_docs"),
            ("kbVectors", "kb_vectors"),
        ];
        TABLES
            .iter()
            .map(|(key, table)| self.store.table_row_count(table).map(|rows| (*key, rows)))
            .collect()
    }

    /// 清理「缓存」页签勾选的项，返回每项删掉的行数（文件类在 web 层另行处理）。
    pub fn purge_caches(&self, keys: &[String]) -> Result<Vec<(String, u64)>> {
        let mut out = Vec::new();
        for key in keys {
            let cleared = match key.as_str() {
                "schemaCache" => self.clear_all_schema_cache()? as u64,
                "queryHistory" => self.store.clear_history()? as u64,
                "aiAudit" => self.store.purge_table("ai_audit")? as u64,
                "aiUsageDays" => self.store.purge_table("ai_usage_days")? as u64,
                "aiUsageModels" => self.store.purge_table("ai_usage_models")? as u64,
                "kbDocs" => self.store.purge_table("kb_docs")? as u64,
                "kbVectors" => self.store.purge_table("kb_vectors")? as u64,
                // 文件类（backups / exports / logs）不归内核管，web 层跳过并原样回显
                other => {
                    out.push((other.to_string(), 0));
                    continue;
                }
            };
            out.push((key.clone(), cleared));
        }
        Ok(out)
    }

    /// 把「SSH 隧道空闲回收时长」设置同步给隧道层（sweeper 读静态原子量）。
    ///
    /// 30 分钟曾经写死在 `tunnel.rs` —— 云库/RDS 的连接配额金贵，内网库又想放久点，
    /// 没有统一答案，那就交给设置（`tunnel.idleTimeoutSecs`，0 = 不按空闲回收）。
    fn apply_tunnel_idle_timeout(&self) {
        let secs = self
            .store
            .get_usize_setting(Store::KEY_TUNNEL_IDLE_TIMEOUT, 1800)
            .unwrap_or(1800)
            .min(86_400) as u64;
        crate::tunnel::set_idle_timeout_secs(secs);
    }

    // ------------------------------------------------------------ 历史与设置

    pub fn history(&self, limit: usize, connection: Option<&str>) -> Result<Vec<HistoryEntry>> {
        let connection_id = match connection {
            Some(key) => Some(self.store.require_connection(key)?.id),
            None => None,
        };
        self.store.list_history(limit, connection_id.as_deref())
    }

    pub fn clear_history(&self) -> Result<usize> {
        self.store.clear_history()
    }

    pub fn settings(&self) -> Result<Vec<(String, String)>> {
        self.store.all_settings()
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        self.store.get_setting(key)
    }

    /// 写设置并把安全相关项**立刻**同步到运行中的策略。
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.store.set_setting(key, value)?;
        // 改的是会话相关设置 ⇒ 立刻生效（不等重启）
        if key == Store::KEY_SESSION_MAX_PER_HOST || key == Store::KEY_SESSION_IDLE_TIMEOUT {
            self.apply_session_limits()?;
        }
        let flag = value == "true" || value == "1";
        match key {
            Store::KEY_PROTECT_PRODUCTION => self.update_policy(|p| p.protect_production = flag),
            Store::KEY_AI_WRITE_ENABLED => self.update_policy(|p| p.ai_write_enabled = flag),
            Store::KEY_BLOCK_DANGEROUS => self.update_policy(|p| p.block_dangerous = flag),
            Store::KEY_MAX_WRITE_ROWS => {
                let max = value.parse::<u64>().unwrap_or(0);
                self.update_policy(|p| p.max_write_rows = max);
            }
            // 下一次拉起宿主 JVM 时生效（宿主是长驻进程，已在跑的不受影响）
            Store::KEY_ALLOW_LEGACY_TLS => crate::agent::set_legacy_tls_enabled(flag),
            // 隧道空闲回收：sweeper 每轮读静态原子量，改完下一轮就生效
            Store::KEY_TUNNEL_IDLE_TIMEOUT => self.apply_tunnel_idle_timeout(),
            // 保留策略改了立刻清一遍：用户把 100000 改成 100，不该等到下次执行才看到
            Store::KEY_HISTORY_MAX_ENTRIES | Store::KEY_HISTORY_RETENTION_DAYS => {
                self.prune_history();
            }
            _ => {}
        }
        Ok(())
    }

    /// 驱动与 agent 的就绪报告。
    ///
    /// 存在的意义：把「未接入（DRV-0001）」和「依赖缺（DRV-0002）」在**UI 上分开**。
    /// 用户看到「缺驱动」时应该能直接看到坐标与安装命令，而不是去猜。
    pub fn driver_report(&self) -> DriverReport {
        let entries: Vec<DriverEntry> = ConnectionKind::ALL
            .iter()
            .copied()
            .filter(|kind| kind.runtime_mode() == RuntimeMode::Agent)
            .map(|kind| {
                let agent_key = kind.agent_key();
                let host = self.drivers.host_for(kind);
                // 驱动 jar 只有 JDBC 类型需要（专属宿主自带驱动库）
                let jar_count = if kind.is_jdbc() {
                    agent_key
                        .map(|key| crate::installed_driver_jars(key).len())
                        .unwrap_or(0)
                } else {
                    0
                };
                DriverEntry {
                    kind,
                    key: kind.key().to_string(),
                    label: kind.label().to_string(),
                    agent_key: agent_key.map(|s| s.to_string()),
                    artifact: kind.jdbc_artifact().map(|s| s.to_string()),
                    jdbc: kind.is_jdbc(),
                    host: host.as_ref().map(|h| h.spec().id.to_string()),
                    installed: if kind.is_jdbc() {
                        jar_count > 0
                    } else {
                        host.is_some()
                    },
                    jar_count,
                    directory: agent_key
                        .map(|key| crate::driver_dir(key).display().to_string())
                        .unwrap_or_default(),
                }
            })
            .collect();
        let installed_count = entries.iter().filter(|entry| entry.installed).count();
        DriverReport {
            agents: crate::agent::availability_all(),
            entries,
            installed_count,
        }
    }

    /// 运行时摘要，便于壳层展示「当前处于什么模式」。
    pub fn runtime_summary(&self) -> RuntimeSummary {
        let policy = self.policy();
        let report = self.driver_report();
        let ready_hosts = report.agents.iter().filter(|a| a.ready).count();
        RuntimeSummary {
            store_path: self.store.path().display().to_string(),
            connection_count: self.store.list_connections().map(|v| v.len()).unwrap_or(0),
            protect_production: policy.protect_production,
            ai_write_enabled: policy.ai_write_enabled,
            implemented_types: ConnectionKind::ALL
                .iter()
                .copied()
                .filter(|k| self.drivers.implemented(*k))
                .count(),
            declared_types: ConnectionKind::ALL.len(),
            native_kinds: ConnectionKind::ALL
                .iter()
                .copied()
                .filter(|k| k.runtime_mode() == RuntimeMode::Native)
                .collect(),
            agents_ready: ready_hosts,
            agents_total: report.agents.len(),
            agent_ready: ready_hosts == report.agents.len() && !report.agents.is_empty(),
            agent_reason: report
                .agents
                .iter()
                .find(|a| !a.ready)
                .and_then(|a| a.reason.clone()),
            installed_agent_keys: report.installed_count,
        }
    }
}

impl Drop for DbMindEngine {
    fn drop(&mut self) {
        // 内核退出时收掉全部 agent 进程：否则会留下孤儿 java 进程占着内存
        for host in self.drivers.hosts() {
            host.shutdown();
        }
    }
}

impl DbMindEngine {
    /// 回收全部 agent 宿主进程（下次用到时按需重新拉起）。
    ///
    /// 为什么需要它：**JVM 的 classpath 在启动那一刻就固定了**。驱动 jar 是分几次下载的
    /// （ClickHouse 的 `all` 包 8.1 MB 与 `slf4j-api` 就差 1 秒先后落盘），
    /// 夹在中间起来的宿主就少了后下的那个 jar，而它会一直活着 ——
    /// 表现为「目录里明明有 slf4j，却报 `NoClassDefFoundError: org/slf4j/LoggerFactory`」
    /// （实测：ClickHouse 的 `listUsers` 报 500，重启宿主即恢复）。
    /// 所以：**驱动文件一变，就把宿主收掉**，让下一次连接重新拉起。
    pub fn recycle_agent_hosts(&self) {
        for host in self.drivers.hosts() {
            host.shutdown();
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSummary {
    pub store_path: String,
    pub connection_count: usize,
    pub protect_production: bool,
    pub ai_write_enabled: bool,
    pub implemented_types: usize,
    pub declared_types: usize,
    pub native_kinds: Vec<ConnectionKind>,
    /// agent 宿主是否**全部**就绪；任一不就绪时 reason 说明缺什么。
    pub agent_ready: bool,
    pub agent_reason: Option<String>,
    /// 已就绪宿主数 / 宿主总数（宿主机可能有 JDBC 与 MongoDB 两类）。
    pub agents_ready: usize,
    pub agents_total: usize,
    /// 已安装驱动的 agentKey 数量。
    pub installed_agent_keys: usize,
}

/// 单个外置驱动条目的就绪状态。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverEntry {
    pub kind: ConnectionKind,
    pub key: String,
    pub label: String,
    pub agent_key: Option<String>,
    /// Maven 坐标（来自 YAML），缺驱动时用它提示安装。
    pub artifact: Option<String>,
    /// 是否走 JDBC 宿主（false = 由专属协议宿主承载，如 MongoDB）。
    pub jdbc: bool,
    /// 承载该类型的宿主 id（jdbc / mongodb …）；None = 还没有宿主。
    pub host: Option<String>,
    pub installed: bool,
    pub jar_count: usize,
    /// 该 agentKey 的驱动目录（提示用户往哪放）。
    pub directory: String,
}

/// 驱动与 agent 宿主的总报告。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriverReport {
    /// 各宿主的就绪状态（JDBC / MongoDB …）
    pub agents: Vec<crate::AgentAvailability>,
    pub entries: Vec<DriverEntry>,
    pub installed_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            static SEQ: AtomicU64 = AtomicU64::new(0);
            let unique = format!(
                "dbmind-test-{tag}-{}-{}",
                std::process::id(),
                SEQ.fetch_add(1, Ordering::Relaxed)
            );
            let dir = std::env::temp_dir().join(unique);
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("创建临时目录失败");
            Self(dir)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// 建一个指向临时 SQLite 文件的连接，返回 (临时目录, 引擎, 连接 id)。
    fn fixture(tag: &str) -> (TempDir, DbMindEngine, String) {
        let dir = TempDir::new(tag);
        let engine = DbMindEngine::open(&dir.path("dbmind.db")).expect("打开引擎失败");
        let record = engine
            .add_connection(
                ConnectionConfig::new("local", ConnectionKind::Sqlite)
                    .with_file(dir.path("data.db").to_string_lossy().to_string()),
            )
            .expect("添加连接失败");
        (dir, engine, record.id)
    }

    /// 全局会话配额要**真的管到原生驱动**（否则「全局」这句话有个说不出口的例外）：
    /// 设成 1 之后，两个文件型 SQLite 连接会互相挤掉对方的**空闲**会话 ——
    /// 连接级状态（临时表）随之丢失，磁盘上的数据不受影响。
    #[test]
    fn 全局会话配额对原生驱动同样生效() {
        let dir = TempDir::new("quota-native");
        let engine = DbMindEngine::open(&dir.path("dbmind.db")).expect("打开引擎失败");
        let a = engine
            .add_connection(
                ConnectionConfig::new("a", ConnectionKind::Sqlite)
                    .with_file(dir.path("a.db").to_string_lossy().to_string()),
            )
            .expect("添加连接失败");
        let b = engine
            .add_connection(
                ConnectionConfig::new("b", ConnectionKind::Sqlite)
                    .with_file(dir.path("b.db").to_string_lossy().to_string()),
            )
            .expect("添加连接失败");

        engine
            .set_setting(Store::KEY_SESSION_MAX_PER_HOST, "1")
            .expect("写设置失败");

        // 断言要看 detail：驱动的具体原因（如 "no such table"）在那里，
        // `message` 是统一的「SQL 执行失败」（Display 只给 message + 错误码）
        let run = |id: &str, sql: &str| {
            engine
                .execute(QueryRequest::new(id, sql), AccessContext::Desktop)
                .map(|_| ())
                .map_err(|e| format!("{}（{}）", e, e.detail.as_deref().unwrap_or("-")))
        };

        run(&a.id, "create temp table tt(x integer)").expect("建临时表应成功");
        run(&b.id, "select 1").expect("第二条连接应当开得起来（先回收空闲会话）");

        // 配额 = 1：A 的会话在 B 打开时被回收过 ⇒ A 重开后临时表没了
        let err = run(&a.id, "select * from tt").expect_err("临时表应随会话一起消失");
        assert!(err.contains("no such table"), "{err}");

        // 磁盘上的数据不受影响（这是我们敢回收文件型连接的前提）
        run(&a.id, "create table persisted(x integer)").expect("建表应成功");
        run(&a.id, "insert into persisted values (1)").expect("写入应成功");
        run(&a.id, "select count(*) as n from persisted").expect("读取应成功");
    }

    /// 空闲回收的**接线**：设置 → 预算。清理线程本身只是「睡 10 秒再调同一段逻辑」，
    /// 所以这里直接按截止时刻调它调的那个函数（不必真等一个扫描周期）。
    #[test]
    fn 空闲回收设置能到达预算() {
        let (_dir, engine, conn) = fixture("idle-sweep");
        engine
            .set_setting(Store::KEY_SESSION_IDLE_TIMEOUT, "1")
            .expect("写设置失败");
        let budget = engine.session_budget();
        assert_eq!(
            budget.idle_timeout_secs(),
            1,
            "设置要到达预算 —— 否则线程扫的仍然是 0（等于没开）"
        );

        engine
            .execute(QueryRequest::new(&conn, "select 1"), AccessContext::Desktop)
            .expect("查询应成功");
        assert_eq!(budget.live(), 1);

        // 用「未来」的截止时刻代表「已经闲置很久」（不真等 10 秒的扫描周期）
        let cutoff = std::time::Instant::now() + std::time::Duration::from_secs(1);
        assert_eq!(budget.sweep_older_than(cutoff), 1, "空闲会话应被归还");
        assert_eq!(budget.live(), 0);

        // 归还之后照样能用：下一条语句自动重开
        engine
            .execute(QueryRequest::new(&conn, "select 1"), AccessContext::Desktop)
            .expect("重开应成功");
        assert_eq!(budget.live(), 1, "重开后重新占一个名额");
    }

    /// 结构缓存：命中时不回查数据库，`_fresh` 能绕过。
    ///
    /// 用「另一个连接直接改库」来制造外部变更 —— 这是唯一能证明
    /// 「缓存真的生效」而不是「碰巧结果一样」的做法。
    #[test]
    fn 结构缓存命中且可绕过() {
        let (dir, engine, conn) = fixture("schema-cache");
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table t1(a integer)"),
                AccessContext::Desktop,
            )
            .unwrap();

        let first = engine.list_tables(&conn).unwrap();
        assert!(first.iter().any(|t| t.name == "t1"));
        assert!(
            engine.schema_cache_info(&conn).unwrap().entries > 0,
            "浏览结构应写入缓存"
        );

        // 绕过引擎直接改库：模拟「同事/别的进程改了结构」
        let raw = rusqlite::Connection::open(dir.path("data.db")).unwrap();
        raw.execute_batch("create table t2(b integer)").unwrap();

        let cached = engine.list_tables(&conn).unwrap();
        assert!(!cached.iter().any(|t| t.name == "t2"), "缓存期内不该回查数据库");

        let fresh = engine.list_tables_fresh(&conn).unwrap();
        assert!(fresh.iter().any(|t| t.name == "t2"), "_fresh 必须绕过缓存");
    }

    /// DDL 是引擎内唯一的失效信号：同一个会话里建表后必须立刻可见，
    /// 不能等 TTL（用户会以为语句没生效）。
    #[test]
    fn 结构变更立刻作废缓存() {
        let (_dir, engine, conn) = fixture("schema-cache-ddl");
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table t1(a integer)"),
                AccessContext::Desktop,
            )
            .unwrap();
        let _ = engine.list_tables(&conn).unwrap();
        assert!(engine.schema_cache_info(&conn).unwrap().entries > 0);

        engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table t2(b integer)"),
                AccessContext::Desktop,
            )
            .unwrap();
        assert_eq!(
            engine.schema_cache_info(&conn).unwrap().entries,
            0,
            "DDL 应清空该连接的缓存"
        );

        let tables = engine.list_tables(&conn).unwrap();
        assert!(tables.iter().any(|t| t.name == "t2"), "DDL 后应立刻看到新表");
    }

    /// 列缓存与清单缓存各自独立：缓存键不同，作废也不会互相误伤。
    #[test]
    fn 列缓存与清单缓存独立() {
        let (_dir, engine, conn) = fixture("schema-cache-columns");
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table t1(a integer, b text)"),
                AccessContext::Desktop,
            )
            .unwrap();
        let columns = engine.list_columns(&conn, "t1").unwrap();
        assert_eq!(columns.len(), 2);
        // 只读过列 ⇒ 只应有一条缓存
        assert_eq!(engine.schema_cache_info(&conn).unwrap().entries, 1);

        assert_eq!(engine.clear_schema_cache(&conn).unwrap(), 1);
        assert_eq!(engine.schema_cache_info(&conn).unwrap().entries, 0);
    }

    /// 改连接配置必须作废缓存：旧缓存描述的可能**是另一个库**。
    /// 编辑连接时**不带口令**，不能把已存口令清掉。
    ///
    /// 场景是必然发生的：详情接口只写不读（`sanitize` 不回传口令），
    /// 所以界面上「编辑 → 保存」交回来的配置里 `password` 一定是 `None`。
    /// 早先的实现把它原样写库 ⇒ 改一下库名就把口令清成 NULL，连接随即失效。
    /// 约定：`None` = 沿用库里那份；空串 = 真的要清掉。
    #[test]
    fn 改连接不带口令时保留原口令() {
        let (_dir, engine, _conn) = fixture("keep-password");
        let mut config = ConnectionConfig::new("keep-pw", ConnectionKind::Mysql)
            .with_host("127.0.0.1")
            .with_port(3306);
        config.password = Some("s3cret".to_string());
        let created = engine.add_connection(config).expect("建连接失败");

        // 界面交回来的那份：没有口令（拿不到明文）
        let edit = ConnectionConfig::new("keep-pw-renamed", ConnectionKind::Mysql)
            .with_host("127.0.0.1")
            .with_port(3307);
        let updated = engine.update_connection(&created.id, edit).expect("更新连接失败");
        assert_eq!(updated.name(), "keep-pw-renamed", "改的连接名要生效");
        assert_eq!(updated.config.port, Some(3307), "改的端口要生效");
        assert_eq!(
            updated.config.password.as_deref(),
            Some("s3cret"),
            "编辑连接不该把已存口令清掉（前端拿不到明文，必然交回 None）"
        );

        // 显式给空串 = 真的要清掉（否则用户没有任何办法删掉一个存错的口令）
        let mut clear = ConnectionConfig::new("keep-pw-renamed", ConnectionKind::Mysql)
            .with_host("127.0.0.1")
            .with_port(3307);
        clear.password = Some(String::new());
        let cleared = engine.update_connection(&created.id, clear).expect("清口令失败");
        assert_eq!(
            cleared.config.password.as_deref(),
            Some(""),
            "给了空串就该真的清掉"
        );
    }

    #[test]
    fn 改连接配置后缓存作废() {
        let (dir, engine, conn) = fixture("schema-cache-reconf");
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table t1(a integer)"),
                AccessContext::Desktop,
            )
            .unwrap();
        let _ = engine.list_tables(&conn).unwrap();
        assert!(engine.schema_cache_info(&conn).unwrap().entries > 0);

        // 目标库得先存在：SQLite 只读打开一个不存在的文件会如实报「打开失败」，
        // 那是连接配置问题，不该由缓存测试来掩盖
        let other_path = dir.path("other.db");
        rusqlite::Connection::open(&other_path)
            .unwrap()
            .execute_batch("create table unrelated(x integer)")
            .unwrap();

        let record = engine.require_connection(&conn).unwrap();
        engine
            .update_connection(
                &record.id,
                record
                    .config
                    .clone()
                    .with_file(other_path.to_string_lossy().to_string()),
            )
            .unwrap();

        assert_eq!(
            engine.schema_cache_info(&conn).unwrap().entries,
            0,
            "换了目标库，旧缓存必须清掉"
        );
        let tables = engine.list_tables_fresh(&conn).unwrap();
        assert!(
            tables.iter().any(|t| t.name == "unrelated") && !tables.iter().any(|t| t.name == "t1"),
            "刷新后应看到新库的结构：{:?}",
            tables.iter().map(|t| t.name.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn 端到端建表写入查询() {
        let (_dir, engine, conn) = fixture("e2e");

        let created = engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table t(id integer primary key, name text)"),
                AccessContext::Desktop,
            )
            .unwrap();
        assert_eq!(created.statement_kind, crate::StatementKind::Ddl);

        let inserted = engine
            .execute(
                QueryRequest::new(
                    conn.as_str(),
                    "insert into t (id, name) values (1, 'a'), (2, 'b')",
                ),
                AccessContext::Desktop,
            )
            .unwrap();
        assert_eq!(inserted.affected_rows, Some(2));

        let result = engine
            .execute(
                QueryRequest::new(conn.as_str(), "select id, name from t order by id"),
                AccessContext::Desktop,
            )
            .unwrap();
        assert_eq!(result.row_count, 2);
        assert_eq!(result.columns.len(), 2);
        assert_eq!(result.columns[0].name, "id");
        assert_eq!(result.rows[0][0], CellValue::Integer(1));
        assert_eq!(result.rows[1][1], CellValue::Text("b".to_string()));
        assert!(!result.truncated);
        assert_eq!(result.connection_id, conn);
        assert!(!result.execution_id.is_empty());
    }

    /// `:memory:` 是**连接级**作用域：声明了单连接的类型（sqlite/h2/derby/duckdb…）
    /// 必须在同一条连接里执行语句，否则「建表 → 插入 → 查询」三步各自看到一个空库
    /// —— 实测复现过：建表成功，紧接着 `select` 说 no such table。
    #[test]
    fn 内存库的语句走同一条连接() {
        let (_dir, engine, _conn) = fixture("memory");
        let conn = engine
            .add_connection(ConnectionConfig::new("mem", ConnectionKind::Sqlite).with_file(":memory:"))
            .unwrap();

        engine
            .execute(
                QueryRequest::new(conn.id.as_str(), "create table t(a integer)"),
                AccessContext::Desktop,
            )
            .unwrap();
        engine
            .execute(
                QueryRequest::new(conn.id.as_str(), "insert into t values (1), (2)"),
                AccessContext::Desktop,
            )
            .unwrap();
        let result = engine
            .execute(
                QueryRequest::new(conn.id.as_str(), "select count(*) as n from t"),
                AccessContext::Desktop,
            )
            .unwrap();
        assert_eq!(result.rows[0][0], CellValue::Integer(2));

        // 结构浏览也看得见 ⇒ 元数据搭的就是那条连接
        let tables = engine.list_tables_fresh(conn.id.as_str()).unwrap();
        assert!(
            tables.iter().any(|item| item.name == "t"),
            "结构浏览应看到内存库里的表：{tables:?}"
        );
    }

    /// 就地编辑的前提：内核要能说清「这份结果来自哪张表」。
    ///
    /// 判不出来必须给 None —— 壳层据此**不给**编辑入口，而不是自己去猜一个表名
    /// （猜错就会改错行）。
    #[test]
    fn 读结果的来源对象() {
        let (_dir, engine, conn) = fixture("source");
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table t(id integer primary key, name text)"),
                AccessContext::Desktop,
            )
            .unwrap();
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "insert into t (id, name) values (1, 'a')"),
                AccessContext::Desktop,
            )
            .unwrap();

        let read = engine
            .execute(
                QueryRequest::new(conn.as_str(), "select id, name from t where id = 1"),
                AccessContext::Desktop,
            )
            .unwrap();
        assert_eq!(read.source_object.as_deref(), Some("t"));

        // 聚合结果不是「某张表的行」
        let aggregated = engine
            .execute(
                QueryRequest::new(conn.as_str(), "select count(*) from t"),
                AccessContext::Desktop,
            )
            .unwrap();
        assert_eq!(aggregated.source_object, None);

        // 写语句即使有对象名，也没有「可改的行」
        let written = engine
            .execute(
                QueryRequest::new(conn.as_str(), "update t set name = 'b' where id = 1"),
                AccessContext::Desktop,
            )
            .unwrap();
        assert_eq!(written.source_object, None);

        // 多表 JOIN 同样判不出来
        let joined = engine
            .execute(
                QueryRequest::new(conn.as_str(), "select a.id from t a join t b on a.id = b.id"),
                AccessContext::Desktop,
            )
            .unwrap();
        assert_eq!(joined.source_object, None);
    }

    #[test]
    fn 超过页大小会被截断并给出提示() {
        let (_dir, engine, conn) = fixture("truncate");
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table n(v integer)"),
                AccessContext::Desktop,
            )
            .unwrap();
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "insert into n(v) values (1),(2),(3),(4),(5)"),
                AccessContext::Desktop,
            )
            .unwrap();

        let mut request = QueryRequest::new(conn.as_str(), "select v from n order by v");
        request.options = request.options.with_max_rows(2);
        let result = engine.execute(request, AccessContext::Desktop).unwrap();

        assert_eq!(result.row_count, 2);
        assert!(result.truncated);
        assert_eq!(result.notices.len(), 1);
    }

    #[test]
    fn 只读连接拒绝写但允许读() {
        let (_dir, engine, conn) = fixture("readonly");
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table t(a integer)"),
                AccessContext::Desktop,
            )
            .unwrap();
        engine.set_read_only(&conn, true).unwrap();

        let err = engine
            .execute(
                QueryRequest::new(conn.as_str(), "insert into t values (1)"),
                AccessContext::Desktop,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyReadOnly);

        // 读仍然可用
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "select count(*) from t"),
                AccessContext::Desktop,
            )
            .unwrap();

        // 解除后可以写
        engine.set_read_only(&conn, false).unwrap();
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "insert into t values (1)"),
                AccessContext::Desktop,
            )
            .unwrap();
    }

    /// 影子连接（跨库浏览按需生成的记录）的只读标记必须**始终跟主连接一致**。
    ///
    /// 这条钉的是实机撞到的一幕：数据源上开了「只读」，从库节点执行 `insert` 照样写得进去 ——
    /// 因为语句是在影子身上跑的，而影子是独立记录、只读标记默认 false。
    #[test]
    fn 影子连接的只读跟着主连接() {
        let (_dir, engine, base) = fixture("shadow-readonly");
        engine
            .execute(
                QueryRequest::new(base.as_str(), "create table t(a integer)"),
                AccessContext::Desktop,
            )
            .unwrap();
        // 跨库浏览造出的那条记录：同配置、只换库名 + 打上 `shadowFor` 标记
        // （web 层的 `scope::ensure_shadow` 就是这么做的）
        let shadow_config = || {
            let mut config = engine.require_connection(&base).unwrap().config.clone();
            config.name = "local ▸ other".to_string();
            config.database = Some("other".to_string());
            let mut extra = match config.extra.take() {
                Some(serde_json::Value::Object(map)) => map,
                _ => serde_json::Map::new(),
            };
            extra.insert(
                EXTRA_SHADOW_FOR.to_string(),
                serde_json::json!(base.clone()),
            );
            config.extra = Some(serde_json::Value::Object(extra));
            config
        };
        let write = |conn: &str| {
            engine.execute(
                QueryRequest::new(conn, "insert into t values (1)"),
                AccessContext::Desktop,
            )
        };

        // ① 主连接先开只读，**之后**出生的影子要继承（否则点开一个新库就等于绕过闸门）
        engine.set_read_only(&base, true).unwrap();
        let shadow = engine.add_connection(shadow_config()).unwrap();
        assert!(shadow.is_shadow(), "记录上打了 shadowFor 就该被认成影子");
        assert!(shadow.read_only, "影子出生就该继承主连接的只读标记");
        assert_eq!(
            write(&shadow.id).unwrap_err().code,
            ErrorCode::SafetyReadOnly,
            "影子上的写必须被拦下"
        );

        // ② 已经存在的影子，开关打开时要跟着改 —— 这正是用户撞到的顺序：先点开库，再开只读
        engine.set_read_only(&base, false).unwrap();
        assert!(
            !engine.require_connection(&shadow.id).unwrap().read_only,
            "关掉开关时已有影子也要跟着清"
        );
        engine.set_read_only(&base, true).unwrap();
        assert!(
            engine.require_connection(&shadow.id).unwrap().read_only,
            "开关打开时已有影子必须跟着改"
        );
        assert_eq!(write(&shadow.id).unwrap_err().code, ErrorCode::SafetyReadOnly);

        // ③ 关掉之后要能真的写进去（只读一旦打开就关不掉是另一种坏）
        engine.set_read_only(&base, false).unwrap();
        assert!(write(&shadow.id).is_ok(), "解除只读后影子应当可以写");

        // 非影子的连接不受影响：开只读不该波及别人
        let other = engine
            .add_connection(
                ConnectionConfig::new("plain", ConnectionKind::Sqlite)
                    .with_file(_dir.path("plain.db").to_string_lossy().to_string()),
            )
            .unwrap();
        engine.set_read_only(&base, true).unwrap();
        assert!(
            !engine.require_connection(&other.id).unwrap().read_only,
            "只读标记只能落在主连接与它自己的影子上"
        );
    }

    /// 老库里「主连接只读、影子没跟上」的记录，**开机时要被自动对齐**。
    ///
    /// 只靠「下次切换开关时顺带修正」等于要求用户先关掉再打开一次，而他要的只是
    /// 「现在就该拦住」—— 这个不变量是后来才补上的，已经存在的数据得有人管。
    #[test]
    fn 开机把影子的只读标记与主连接对齐() {
        let dir = TempDir::new("shadow-reconcile");
        let store_path = dir.path("dbmind.db");
        let shadow_id = {
            // 直接用存储层造数据：模拟旧版本留下的影子 —— 它的 read_only 停在 false
            let store = Store::open(&store_path).expect("打开存储失败");
            let base = store
                .insert_connection(
                    &ConnectionConfig::new("local", ConnectionKind::Sqlite)
                        .with_file(dir.path("data.db").to_string_lossy().to_string()),
                )
                .expect("插入主连接失败");
            store.set_read_only(&base.id, true).expect("标记只读失败");
            let mut extra = serde_json::Map::new();
            extra.insert(EXTRA_SHADOW_FOR.to_string(), serde_json::json!(base.id));
            let mut config = base.config.clone();
            config.name = "local ▸ other".to_string();
            config.extra = Some(serde_json::Value::Object(extra));
            let shadow = store.insert_connection(&config).expect("插入影子失败");
            assert!(!shadow.read_only, "旧数据就长这样：主连接只读、影子可写");
            shadow.id
        };

        let engine = DbMindEngine::open(&store_path).expect("打开引擎失败");
        assert!(
            engine.require_connection(&shadow_id).unwrap().read_only,
            "开机应当把影子的只读标记对齐到主连接"
        );
        assert_eq!(
            engine
                .execute(
                    QueryRequest::new(shadow_id.as_str(), "insert into t values (1)"),
                    AccessContext::Desktop,
                )
                .unwrap_err()
                .code,
            ErrorCode::SafetyReadOnly,
            "对齐之后闸门必须真的拦得住"
        );
    }

    #[test]
    fn ai_通道默认只读且可通过设置放开() {
        let (_dir, engine, conn) = fixture("ai");
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table t(a integer)"),
                AccessContext::Desktop,
            )
            .unwrap();

        let err = engine
            .execute(
                QueryRequest::new(conn.as_str(), "insert into t values (1)"),
                AccessContext::Mcp,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyAiReadOnly);

        engine.set_setting(Store::KEY_AI_WRITE_ENABLED, "true").unwrap();
        assert!(engine.policy().ai_write_enabled);
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "insert into t values (1)"),
                AccessContext::Mcp,
            )
            .unwrap();
    }

    #[test]
    fn 生产保护拦截生产环境的一切写() {
        let (_dir, engine, conn) = fixture("prod");
        // 生产保护**只对标注为「生产」环境的数据源生效**：给这条连接打上 PROD 标
        let record = engine
            .find_connection(&conn)
            .unwrap()
            .expect("fixture 连接应存在");
        let mut config = record.config.clone();
        config.extra = Some(serde_json::json!({ "group": "PROD" }));
        engine.update_connection(&conn, config).unwrap();
        engine.set_setting(Store::KEY_PROTECT_PRODUCTION, "true").unwrap();
        let err = engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table t(a integer)"),
                AccessContext::Desktop,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyProduction);
        // 未标注环境的连接不受生产保护影响（写操作正常执行）
        let (_dir2, engine2, conn2) = fixture("dev");
        engine2
            .set_setting(Store::KEY_PROTECT_PRODUCTION, "true")
            .unwrap();
        engine2
            .execute(
                QueryRequest::new(conn2.as_str(), "create table t(a integer)"),
                AccessContext::Desktop,
            )
            .unwrap();
    }

    /// 影子连接的**安全属性跟随主连接**：跨库浏览生成的影子揣着创建那一刻的旧标注，
    /// 主连接后来补标「生产」—— 影子同样要被拦（跨库浏览不是绕过保护的口子，真机踩过）。
    #[test]
    fn 影子连接的安全属性跟随主连接() {
        let (_dir, engine, conn) = fixture("main");
        // 主连接补标生产
        let mut main_cfg = engine
            .find_connection(&conn)
            .unwrap()
            .expect("主连接存在")
            .config;
        main_cfg.extra = Some(serde_json::json!({ "env": "PROD", "group": "PROD" }));
        engine.update_connection(&conn, main_cfg).unwrap();

        // 更早生成的影子：揣着旧角标 DEV
        let mut shadow_cfg = engine
            .find_connection(&conn)
            .unwrap()
            .unwrap()
            .config
            .clone();
        shadow_cfg.name = format!("{conn} ▸ db2");
        shadow_cfg.extra = Some(serde_json::json!({
            "shadowFor": conn,
            "shadowDatabase": "db2",
            "env": "DEV"
        }));
        let shadow = engine.add_connection(shadow_cfg).unwrap();

        engine
            .set_setting(Store::KEY_PROTECT_PRODUCTION, "true")
            .unwrap();
        let err = engine
            .execute(
                QueryRequest::new(shadow.id.as_str(), "create table t(a integer)"),
                AccessContext::Desktop,
            )
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyProduction);
    }

    /// 危险语句拦截：无 WHERE 的 UPDATE/DELETE、TRUNCATE、DROP 被闸门拒绝。
    #[test]
    fn 危险语句拦截() {
        let (_dir, engine, conn) = fixture("danger");
        engine
            .set_setting(Store::KEY_BLOCK_DANGEROUS, "true")
            .unwrap();
        for sql in ["delete from t", "update t set a = 1", "drop table t"] {
            let err = engine
                .execute(QueryRequest::new(conn.as_str(), sql), AccessContext::Desktop)
                .unwrap_err();
            assert_eq!(err.code, ErrorCode::SafetyDangerous, "sql: {sql}");
        }
        // 带 WHERE 的正常写不受影响（会走到驱动层，SQLite 文件驱动可用）
        engine
            .execute(QueryRequest::new(conn.as_str(), "create table t(a integer)"), AccessContext::Desktop)
            .unwrap();
        // 关掉开关后同一条语句放行
        engine.set_setting(Store::KEY_BLOCK_DANGEROUS, "false").unwrap();
        engine
            .execute(QueryRequest::new(conn.as_str(), "drop table t"), AccessContext::Desktop)
            .unwrap();
    }

    /// 写操作影响行数上限：预估超过上限拒绝；预估解析不了的语句不受影响。
    #[test]
    fn 写操作影响行数上限() {
        let (_dir, engine, conn) = fixture("rowcap");
        engine.execute(
            QueryRequest::new(conn.as_str(), "create table t(a integer)"),
            AccessContext::Desktop,
        ).unwrap();
        for i in 0..5 {
            engine.execute(
                QueryRequest::new(conn.as_str(), format!("insert into t values ({i})")),
                AccessContext::Desktop,
            ).unwrap();
        }
        engine.set_setting(Store::KEY_MAX_WRITE_ROWS, "3").unwrap();
        // 表里 5 行 > 上限 3 ⇒ 拦
        let err = engine
            .execute(QueryRequest::new(conn.as_str(), "delete from t"), AccessContext::Desktop)
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::SafetyRowLimit);
        // WHERE 圈住的行数 ≤ 上限 ⇒ 放行
        engine
            .execute(QueryRequest::new(conn.as_str(), "delete from t where a >= 3"), AccessContext::Desktop)
            .unwrap();
        // 上限调大后同一条全表删除放行
        engine.set_setting(Store::KEY_MAX_WRITE_ROWS, "100").unwrap();
        engine
            .execute(QueryRequest::new(conn.as_str(), "delete from t where a > 0"), AccessContext::Desktop)
            .unwrap();
    }

    /// 专属宿主**不可用**时：报「未就绪 + 怎么补」，而不是「未接入」。
    ///
    /// 两种环境都必须成立 —— 这正是它以前会**假通过**的地方：
    /// - 宿主起不来（缺 java、缺 jar，或 jar 与当前 java 不兼容）⇒ 错误得是「未就绪」并说清缺什么；
    /// - 宿主可用（本机构建过 agents、CI 里也构建）⇒ `PING` 就该**正常返回**：那是功能可用，
    ///   不该被判成失败。
    ///
    /// 原来写死 `.unwrap_err()`，于是「宿主真的能用」反而让测试失败；而它此前之所以通过，
    /// 是因为那几次跑的时候宿主恰好被内存挤崩了 —— 一次**假通过**，比失败更坏。
    ///
    /// 另外：单测默认**不会**真去起宿主（见 `agent::AGENTS_TEST_ENV`）—— 默认落在
    /// 「未就绪」分支且不起 JVM；`DBMIND_TEST_AGENTS=1 cargo test` 才会真的 `PING` 一次。
    #[test]
    fn 专属宿主不可用时给出未就绪而非未接入() {
        let (_dir, engine, _conn) = fixture("host-not-ready");
        let redis = engine
            .add_connection(ConnectionConfig::new("redis", ConnectionKind::Redis).with_host("127.0.0.1"))
            .unwrap();
        match engine.execute(
            QueryRequest::new(redis.id.as_str(), "PING"),
            AccessContext::Desktop,
        ) {
            // 宿主可用：这就是一次真实的（只读）PING
            Ok(result) => assert!(
                !result.rows.is_empty() || result.row_count > 0,
                "宿主可用时 PING 应当返回结果"
            ),
            Err(err) => {
                assert_ne!(
                    err.code,
                    ErrorCode::DriverNotImplemented,
                    "宿主已接入，不该再报「未接入」"
                );
                assert!(
                    matches!(err.code, ErrorCode::DriverNotReady | ErrorCode::ConnConnectFailed),
                    "实际：{} {}",
                    err.code_str(),
                    err.message
                );
                if err.code == ErrorCode::DriverNotReady {
                    let detail = err.detail.unwrap_or_default();
                    assert!(
                        detail.contains("redis") || detail.contains("Java") || detail.contains("宿主"),
                        "未就绪时要告诉用户缺什么：{detail}"
                    );
                }
            }
        }
    }

    /// 非 SQL 协议的关键回归：**闸门在连库之前就该生效**。
    /// 即使 MongoDB 宿主还没构建（DRV-0002），写命令也必须先被只读闸门拦住。
    ///
    /// 单测默认不起宿主（见 `agent::AGENTS_TEST_ENV`），所以这里默认走的是
    /// 「宿主未就绪」那条路径；`DBMIND_TEST_AGENTS=1` 时才会真的去连一次
    /// （那时报的是连接失败）—— **两种都必须先过闸门**。
    #[test]
    fn mongodb_写命令在宿主缺失时也先过闸门() {
        let (_dir, engine, _conn) = fixture("mongo-gate");
        let mongo = engine
            .add_connection(ConnectionConfig::new("mongo", ConnectionKind::Mongodb).with_host("127.0.0.1"))
            .unwrap();

        // 只读连接上的写命令：必须报「安全拦截」，而不是「驱动未就绪」
        engine.set_read_only(&mongo.id, true).unwrap();
        let err = engine
            .execute(
                QueryRequest::new(mongo.id.as_str(), "db.users.deleteMany({})"),
                AccessContext::Desktop,
            )
            .unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::SafetyReadOnly,
            "Mongo 写命令应被闸门拦截，实际：{} {}",
            err.code_str(),
            err.message
        );
        let log = engine.history(5, None).unwrap();
        assert!(
            log.iter()
                .any(|h| h.error_code.as_deref() == Some("DBMIND-SAFETY-0001")),
            "被拦的 Mongo 命令也要留痕"
        );

        // 只读命令则继续往下走，此处应停在「宿主未就绪 / 连接失败」这一层
        let err = engine
            .execute(
                QueryRequest::new(mongo.id.as_str(), "db.users.find({})"),
                AccessContext::Desktop,
            )
            .unwrap_err();
        assert!(
            matches!(err.code, ErrorCode::DriverNotReady | ErrorCode::ConnConnectFailed),
            "只读命令不该被安全层拦，实际：{} {}",
            err.code_str(),
            err.message
        );
        if err.code == ErrorCode::DriverNotReady {
            let detail = err.detail.unwrap_or_default();
            assert!(
                detail.contains("mongodb") || detail.contains("Java") || detail.contains("宿主"),
                "未就绪时要告诉用户缺什么：{detail}"
            );
        }
    }

    /// 单测默认不起宿主（见 `agent::AGENTS_TEST_ENV`）：默认走「未就绪」分支；
    /// 显式打开开关时才会真的去连（`./nowhere/` 那个库甚至会被建出来）。
    #[test]
    fn jdbc_类型在驱动未就绪时给出未就绪而非未接入() {
        let (_dir, engine, _conn) = fixture("jdbc-not-ready");
        let h2 = engine
            .add_connection(
                // 指到一个必然连不上的目标：本用例只关心「驱动未安装」这一层的语义
                ConnectionConfig::new("h2", ConnectionKind::H2).with_file("./nowhere/h2demo"),
            )
            .unwrap();
        // 本机环境决定走到哪一步：没装驱动 ⇒ DRV-0002；装了驱动且 java 可用 ⇒ 连接失败；
        // 而**若目标目录可建、H2 驱动也在**，这条语句甚至会成功（内核会先建父目录）。
        // 三种都不该是「未接入」（DRV-0001）——那会误导用户以为功能没做。
        // 所以这里不能写死 `.unwrap_err()`：那会让「功能其实可用」反而判失败。
        match engine.execute(
            QueryRequest::new(h2.id.as_str(), "select 1"),
            AccessContext::Desktop,
        ) {
            // 驱动真的可用：`select 1` 能通就该通
            Ok(_) => {}
            Err(err) => {
                assert!(
                    matches!(err.code, ErrorCode::DriverNotReady | ErrorCode::ConnConnectFailed),
                    "实际错误码：{} ({})",
                    err.code_str(),
                    err.message
                );
                if err.code == ErrorCode::DriverNotReady {
                    let detail = err.detail.unwrap_or_default();
                    // 与 redis / mongo 两条用例保持同一口径：「宿主」也是可行动的指引
                    // （测试构建不加载宿主时，detail 就是「看不到 jar + 怎么打开宿主」）。
                    // 之前这里只认 drivers / driver fetch，导致同一类消息在两条用例里一过一败。
                    assert!(
                        detail.contains("drivers")
                            || detail.contains("driver fetch")
                            || detail.contains("宿主"),
                        "未就绪时必须告诉用户把驱动放哪：{detail}"
                    );
                }
            }
        }
    }

    /// **宿主层**（默认不跑）：H2 文件库上走一次**完整的跨进程往返**。
    ///
    /// 为什么挑 H2：它不需要任何外部服务（库文件就落在临时目录里），所以这条断言能
    /// 要求「**真的成功**」，而不是「至少没崩」—— 那正是默认层给不了的另一半。
    /// 覆盖：建表 → 写入（含中文）→ 读回 → 结构浏览（元数据必须搭同一条物理会话）→
    /// 失败路径也要是被翻译过的结构化错误，而不是 panic。
    #[test]
    #[ignore = "宿主层：需要真实 JVM（scripts/host-tests.ps1，或 DBMIND_TEST_AGENTS=1 cargo test -p dbmind-core -- --ignored）"]
    fn 宿主层_h2_文件库走一次完整往返() {
        assert!(
            crate::agent::agent_hosts_visible(),
            "这一层要求真实宿主：请设 DBMIND_TEST_AGENTS=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        let (dir, engine, _conn) = fixture("host-layer-h2");
        let h2 = engine
            .add_connection(
                ConnectionConfig::new("h2", ConnectionKind::H2)
                    .with_file(dir.path("host-layer").to_string_lossy().to_string()),
            )
            .expect("添加 H2 连接失败");

        let run = |sql: &str| engine.execute(QueryRequest::new(h2.id.as_str(), sql), AccessContext::Desktop);

        run("create table t(id int primary key, name varchar(20))")
            .expect("建表应当成功 —— 这一层的前提就是宿主真的可用");
        let inserted = run("insert into t values (1, '中文名字')").expect("写入应当成功");
        assert_eq!(inserted.affected_rows, Some(1), "插入 1 行要如实报回影响行数");

        let read = run("select name from t where id = 1").expect("读回应当成功");
        assert_eq!(read.rows.len(), 1);
        assert_eq!(
            read.rows[0][0],
            CellValue::Text("中文名字".to_string()),
            "跨进程往返后中文不能坏掉"
        );
        assert!(
            read.session_id.as_deref().is_some_and(|s| !s.is_empty()),
            "结果要带上「跑在哪条物理会话上」"
        );

        // 结构浏览：单连接类型必须搭**数据那条**会话，否则看到的是另一个库（空的）
        let tables = engine
            .list_tables_fresh(h2.id.as_str())
            .expect("结构浏览应当成功");
        // 大小写要小心：H2 把**未加引号**的标识符折成大写，元数据里报的是 `T`。
        // 这是 SQL 标准行为、不是内核的问题 —— 断言按大小写不敏感写，别把这个坑
        // 记成内核缺陷（我第一次就是这么误判的）。
        assert!(
            tables.iter().any(|t| t.name.eq_ignore_ascii_case("t")),
            "结构浏览应当看到刚建的表：{tables:?}"
        );

        // 失败路径：服务端拒绝也要被翻译成结构化错误（不 panic、不误报「未就绪/未接入」）
        let err = run("select * from t where 1 =").unwrap_err();
        assert!(
            matches!(err.code, ErrorCode::QueryFailed | ErrorCode::QueryInvalid),
            "语法错误应是查询类错误，实际：{} {}",
            err.code_str(),
            err.message
        );
        assert!(!err.message.is_empty(), "错误要说人话");
    }

    /// **宿主层**（默认不跑）：取消**真的**打断了一条正在执行的语句。
    ///
    /// 为什么非要用真实宿主：这条链路的每一段都可能「看着像成功」——内核登记表命中、
    /// watcher 线程发出 cancel、宿主 `Statement.cancel()` 打断语句、宿主把 SQL 异常
    /// 翻成 `DBMIND-QUERY-0004`。只有真的跑一条**长语句**再把它掐掉，才能证明整条链路通。
    ///
    /// 做法：故意用一条「不取消就要跑几分钟」的语句（`system_range` + 逐行取模，H2 优化不掉），
    /// 登记后立刻取消 —— 于是**返回得快**本身就是证据。默认 30 秒超时是兜底：取消要是失灵，
    /// 测试会以「超时」而不是「取消」失败 —— 有界，且结论不含糊。
    #[test]
    #[ignore = "宿主层：需要真实 JVM（scripts/host-tests.ps1，或 DBMIND_TEST_AGENTS=1 cargo test -p dbmind-core -- --ignored）"]
    fn 宿主层_取消能真的打断正在执行的语句() {
        assert!(
            crate::agent::agent_hosts_visible(),
            "这一层要求真实宿主：请设 DBMIND_TEST_AGENTS=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        let (dir, engine, _conn) = fixture("host-layer-cancel");
        let h2 = engine
            .add_connection(
                ConnectionConfig::new("h2", ConnectionKind::H2)
                    .with_file(dir.path("cancel").to_string_lossy().to_string()),
            )
            .expect("添加 H2 连接失败");

        let run = |sql: &str| engine.execute(QueryRequest::new(h2.id.as_str(), sql), AccessContext::Desktop);

        // 先证明这条连接**本来就能用**：否则「取消成功」也可能只是「根本没跑起来」
        run("create table t(id int primary key)").expect("建表应当成功");
        let id = DbMindEngine::next_execution_id();
        // 本机实测 2e8 行 ≈ 8.6 秒 ⇒ 4e9 行 ≈ 3 分钟。所以「几秒内返回」只可能是被打断了。
        let slow = "select count(*) from system_range(1, 4000000000) where mod(x, 7) = 0";

        let started = std::time::Instant::now();
        let (outcome, hit) = std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                engine.execute(
                    QueryRequest::new(h2.id.as_str(), slow).with_execution_id(id.clone()),
                    AccessContext::Desktop,
                )
            });
            // 等它真的登记上再取消：否则取消可能打在注册之前，变成「取消了个空」
            let mut waited = std::time::Duration::ZERO;
            let step = std::time::Duration::from_millis(20);
            while waited < std::time::Duration::from_secs(20)
                && !engine.active_executions().iter().any(|e| e == &id)
            {
                std::thread::sleep(step);
                waited += step;
            }
            let hit = engine.cancel(&id);
            (handle.join().expect("查询线程不应 panic"), hit)
        });
        let elapsed = started.elapsed();

        assert!(hit, "取消应当命中正在执行的那条语句");
        let err = outcome.expect_err("被取消的语句不该返回结果");
        assert_eq!(
            err.code,
            ErrorCode::QueryCanceled,
            "被取消应当是 QueryCanceled，实际：{} {} / {}",
            err.code_str(),
            err.message,
            err.detail.as_deref().unwrap_or("-")
        );
        // 把数字打出来（`--nocapture` 可见）：它比「断言过了」更有说服力 ——
        // 这条语句不取消要跑几分钟，而这里只花了几百毫秒。
        println!("取消实测耗时 = {elapsed:?}（这条语句不取消约需 3 分钟）");
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "取消要立刻生效：实测 {elapsed:?}（这条语句不取消要约 3 分钟）"
        );

        // 取消只该掐掉**那条语句**，不该把会话弄坏
        let after = run("select count(*) from t").expect("取消之后会话必须还能用");
        assert_eq!(after.rows[0][0], CellValue::Integer(0), "表还在，且是空的");

        // 反向也要立住：**真超时**必须仍然报「查询超时」。H2 的取消与超时是同一个
        // SQLState（57014），光看异常分不开 —— 所以宿主靠「是不是我们取消的」那份账本
        // 来分，这条断言就是防止那次区分把超时也吞掉。
        let mut timed_out = QueryRequest::new(h2.id.as_str(), slow);
        timed_out.options.timeout_ms = 1_000;
        let err = engine.execute(timed_out, AccessContext::Desktop).unwrap_err();
        assert_eq!(
            err.code,
            ErrorCode::QueryTimeout,
            "真超时应当报查询超时，实际：{} {}",
            err.code_str(),
            err.message
        );

        // 登记表要清干净：否则一次取消会永久占着一个 id
        assert!(
            !engine.active_executions().iter().any(|e| e == &id),
            "执行结束后登记表要清干净：{:?}",
            engine.active_executions()
        );

        // 留痕也要如实：两条语句应当分别记成「已取消」与「失败（超时）」。
        // 这里刻意按 status 各找各的，而不是 `find(system_range)` —— 那个写法会撞上
        // 另一条同 SQL 的记录（我第一次就撞了），断言就变成了「碰运气」。
        let history = engine.history(10, None).expect("读历史失败");
        let slow_entries: Vec<_> = history
            .iter()
            .filter(|h| h.sql.contains("system_range"))
            .collect();
        let cancelled_entry = slow_entries
            .iter()
            .find(|h| h.status.as_str() == "canceled")
            .expect("被取消的语句应当留「已取消」的痕");
        assert_eq!(cancelled_entry.error_code.as_deref(), Some("DBMIND-QUERY-0004"));
        let timeout_entry = slow_entries
            .iter()
            .find(|h| h.status.as_str() == "error")
            .expect("超时的语句应当留「失败」的痕");
        assert_eq!(timeout_entry.error_code.as_deref(), Some("DBMIND-QUERY-0003"));
    }

    /// **宿主层**（默认不跑）：单连接类型上，并发请求**排队**，且共用同一条物理会话。
    ///
    /// 这是「只允许一条物理连接」那句声明的行为兑现：池对它等于一把互斥量。
    /// 断言刻意不靠时序猜测 —— 慢查询先占住会话，1.5 秒后**两条都还活着**（第二条在排队）
    /// 才算数：要是池坏了（真给单连接类型开了第二条），第二条早就返回了。
    #[test]
    #[ignore = "宿主层：需要真实 JVM（scripts/host-tests.ps1，或 DBMIND_TEST_AGENTS=1 cargo test -p dbmind-core -- --ignored）"]
    fn 宿主层_单连接类型的并发请求会排队() {
        assert!(
            crate::agent::agent_hosts_visible(),
            "这一层要求真实宿主：请设 DBMIND_TEST_AGENTS=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        let (dir, engine, _conn) = fixture("host-layer-serial");
        let h2 = engine
            .add_connection(
                ConnectionConfig::new("h2", ConnectionKind::H2)
                    .with_file(dir.path("serial").to_string_lossy().to_string()),
            )
            .expect("添加 H2 连接失败");
        let run = |sql: &str| engine.execute(QueryRequest::new(h2.id.as_str(), sql), AccessContext::Desktop);

        // 先建表：把宿主与连接热起来，顺带拿到本泳道的会话 id 作对照
        let warm = run("create table t(id int primary key)").expect("建表应当成功");
        let lane_session = warm.session_id.expect("结果应当带上会话 id");

        // 一条「不取消要跑几分钟」的语句（同取消测试）：这样「第二条被挡住」的余量极大
        let slow = "select count(*) from system_range(1, 4000000000) where mod(x, 7) = 0";
        let slow_id = DbMindEngine::next_execution_id();

        let (both_queued, hit, fast_elapsed, fast) = std::thread::scope(|scope| {
            let slow_handle = scope.spawn(|| {
                engine.execute(
                    QueryRequest::new(h2.id.as_str(), slow).with_execution_id(slow_id.clone()),
                    AccessContext::Desktop,
                )
            });
            // 给慢查询一点时间**真的占住**那条唯一的会话（宿主已经是热的，毫秒级）
            std::thread::sleep(std::time::Duration::from_millis(400));
            let fast_handle = scope.spawn(|| {
                let started = std::time::Instant::now();
                let result = run("select 1 as x");
                (started.elapsed(), result)
            });

            std::thread::sleep(std::time::Duration::from_millis(1_500));
            let active = engine.active_executions();
            let both_queued = active.len() >= 2 && active.iter().any(|e| e == &slow_id);

            // 取消慢的那条，把两条都放出来（否则这个测试要等几分钟）
            let hit = engine.cancel(&slow_id);
            let (fast_elapsed, fast) = fast_handle.join().expect("查询线程不应 panic");
            let slow = slow_handle.join().expect("查询线程不应 panic");
            assert_eq!(
                slow.unwrap_err().code,
                ErrorCode::QueryCanceled,
                "被取消的慢查询应当报「已取消」"
            );
            (both_queued, hit, fast_elapsed, fast)
        });

        assert!(
            both_queued,
            "等 1.5 秒后两条都应当还在跑（第二条在排队）—— 若第二条已返回，说明池给单连接类型开了不止一条会话"
        );
        assert!(hit, "取消应当命中慢查询");
        println!("排队的第二条等了 {fast_elapsed:?} 才拿到那条唯一的会话（--nocapture 可见）");
        let fast = fast.expect("排队之后那条查询应当成功");
        assert_eq!(fast.rows[0][0], CellValue::Integer(1));
        assert!(
            fast_elapsed >= std::time::Duration::from_millis(1_500),
            "排队的查询要一直等到前面那条让出会话：实测只用了 {fast_elapsed:?}"
        );
        assert_eq!(
            fast.session_id.as_deref(),
            Some(lane_session.as_str()),
            "同一个泳道只有一条会话：排队前与排队后必须是同一条"
        );
    }

    /// **宿主层**（默认不跑）：**排队等会话**的查询也能被立刻取消。
    ///
    /// 单连接类型（H2）只有一条会话：慢查询占住它之后，第二条只能在池里排队。
    /// 此时取消第二条，应当**立刻**回「已取消」，而不是等它拿到会话
    /// （那要等前一条长查询跑完，或等满 30 秒的等待预算）。
    /// 少了这条，用户点了「取消」看到的就是「没反应」。
    #[test]
    #[ignore = "宿主层：需要真实 JVM（scripts/host-tests.ps1，或 DBMIND_TEST_AGENTS=1 cargo test -p dbmind-core -- --ignored）"]
    fn 宿主层_排队等待会话时也能取消() {
        assert!(
            crate::agent::agent_hosts_visible(),
            "这一层要求真实宿主：请设 DBMIND_TEST_AGENTS=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        let (dir, engine, _conn) = fixture("host-layer-queued-cancel");
        let h2 = engine
            .add_connection(
                ConnectionConfig::new("h2", ConnectionKind::H2)
                    .with_file(dir.path("queued").to_string_lossy().to_string()),
            )
            .expect("添加 H2 连接失败");
        let run = |sql: &str| engine.execute(QueryRequest::new(h2.id.as_str(), sql), AccessContext::Desktop);
        // 先热起来：宿主起来了、连接建好了，后面的时序才不会掺进启动开销
        run("create table t(id int primary key)").expect("建表应当成功");

        // 一条「不取消要跑几分钟」的语句：占住唯一那条会话
        let slow = "select count(*) from system_range(1, 4000000000) where mod(x, 7) = 0";
        let slow_id = DbMindEngine::next_execution_id();
        let queued_id = DbMindEngine::next_execution_id();

        let (hit, queued, waited, slow_outcome) = std::thread::scope(|scope| {
            let slow_handle = scope.spawn(|| {
                engine.execute(
                    QueryRequest::new(h2.id.as_str(), slow).with_execution_id(slow_id.clone()),
                    AccessContext::Desktop,
                )
            });
            // 给慢查询时间真的拿到会话
            std::thread::sleep(std::time::Duration::from_millis(400));

            let queued_handle = scope.spawn(|| {
                engine.execute(
                    QueryRequest::new(h2.id.as_str(), slow).with_execution_id(queued_id.clone()),
                    AccessContext::Desktop,
                )
            });
            // 给第二条时间进到池的等待里（宿主是热的，这条路径只有毫秒级）
            std::thread::sleep(std::time::Duration::from_millis(600));

            let started = std::time::Instant::now();
            let hit = engine.cancel(&queued_id);
            let queued = queued_handle.join().expect("查询线程不应 panic");
            let waited = started.elapsed();

            // 收尾：把慢的那条也取消，测试才不会等几分钟
            engine.cancel(&slow_id);
            let slow_outcome = slow_handle.join().expect("查询线程不应 panic");
            (hit, queued, waited, slow_outcome)
        });

        assert!(hit, "取消应当命中排队中的那条");
        let err = queued.expect_err("被取消的排队查询不该返回结果");
        assert_eq!(
            err.code,
            ErrorCode::QueryCanceled,
            "排队被取消应报「已取消」，实际：{} {} / {}",
            err.code_str(),
            err.message,
            err.detail.as_deref().unwrap_or("-")
        );
        assert!(
            err.message.contains("等待可用连接"),
            "要说清是**排队时**被取消（与跑到一半被取消区分开）：{}",
            err.message
        );
        println!("排队中被取消：{waited:?} 就返回了（不修的话要等它拿到会话，即前一条跑完）");
        assert!(
            waited < std::time::Duration::from_secs(3),
            "排队中被取消要立刻返回，不能等它拿到会话：实测 {waited:?}"
        );
        assert_eq!(
            slow_outcome.unwrap_err().code,
            ErrorCode::QueryCanceled,
            "慢查询也要能被取消（测试的收尾靠它）"
        );

        // 取消只掐语句，不该把会话弄坏
        let after = run("select count(*) from t").expect("取消之后会话必须还能用");
        assert_eq!(after.rows[0][0], CellValue::Integer(0));
    }

    /// **宿主层**（默认不跑）：可多连接类型上，并发请求**各占一条物理会话**。
    ///
    /// 需要库测试场（`deploy/mongo-test-server`，`scripts/host-tests.ps1` 会起在 27117）；
    /// 端口可用 `DBMIND_TEST_MONGO_PORT` 覆盖。
    #[test]
    #[ignore = "宿主层：需要真实 JVM + MongoDB 测试场（用 scripts/host-tests.ps1）"]
    fn 宿主层_可多连接类型的并发请求各占一条会话() {
        assert!(
            crate::agent::agent_hosts_visible(),
            "这一层要求真实宿主：请设 DBMIND_TEST_AGENTS=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        let port: u16 = std::env::var("DBMIND_TEST_MONGO_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(27117);
        let (_dir, engine, _conn) = fixture("host-layer-parallel");
        let mongo = engine
            .add_connection(
                ConnectionConfig::new("mongo", ConnectionKind::Mongodb)
                    .with_host("127.0.0.1")
                    .with_port(port)
                    .with_database("hostlayer"),
            )
            .expect("添加 Mongo 连接失败");
        let run =
            |sql: &str| engine.execute(QueryRequest::new(mongo.id.as_str(), sql), AccessContext::Desktop);

        // 先确认真连得上：连不上就该在这里失败，而不是让并发的结论落在「全都连不上」上
        run("db.pings.findOne({})").unwrap_or_else(|e| {
            panic!(
                "连不上库测试场（127.0.0.1:{port}）：{} —— 先跑 scripts/host-tests.ps1",
                e.message
            )
        });

        // 顺序调用：**粘性**（复用上一次用过的那条，会话级状态才不会在语句之间跳）
        let first = run("db.pings.findOne({})").expect("查询失败");
        let second = run("db.pings.findOne({})").expect("查询失败");
        assert_eq!(first.session_id, second.session_id, "顺序调用应当复用同一条会话");

        // 并发：同时发 8 条
        let ids: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| scope.spawn(|| run("db.pings.findOne({})")))
                .collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .expect("查询线程不应 panic")
                        .expect("并发查询应当成功")
                        .session_id
                        .unwrap_or_default()
                })
                .collect()
        });

        let mut lanes = std::collections::BTreeSet::new();
        let mut slots = std::collections::BTreeSet::new();
        for id in &ids {
            let (lane, slot) = id.rsplit_once('#').expect("会话 id 形如 <泳道>#<槽位>");
            lanes.insert(lane.to_string());
            slots.insert(slot.to_string());
        }
        assert_eq!(lanes.len(), 1, "并发请求都在同一条连接（同一个泳道）上：{ids:?}");
        assert!(
            slots.len() >= 2,
            "可多连接类型的并发应当**真的开出第二条**物理会话（惰性生长）：{ids:?}"
        );
        let limit = ConnectionKind::Mongodb.max_connections();
        assert!(
            slots.len() <= limit,
            "槽位数不该超过类型声明的上限 {limit}：{ids:?}"
        );
        println!(
            "并发 8 条落在 {} 条会话上（槽位 {slots:?}，类型上限 {limit}）",
            slots.len()
        );
    }

    /// **宿主层**（默认不跑）：可多连接类型（Elasticsearch）的并发同样各占一条会话。
    ///
    /// 与 Mongo 那条是同一件事的两个协议，**不能只押在一个宿主上**：
    /// `maxConnections` 是内核侧共用机制，但**会话管理与协议实现各宿主独立**
    /// （ES 是 REST 直转宿主，Mongo 是专属协议宿主）。这条把另一半也钉住。
    ///
    /// 需要 ES 替身（`deploy/es-test-server`，`scripts/host-tests.ps1` 会起在 9212）；
    /// 端口可用 `DBMIND_TEST_ES_PORT` 覆盖。
    #[test]
    #[ignore = "宿主层：需要真实 JVM + Elasticsearch 测试替身（用 scripts/host-tests.ps1）"]
    fn 宿主层_可多连接类型的并发请求各占一条会话_es() {
        assert!(
            crate::agent::agent_hosts_visible(),
            "这一层要求真实宿主：请设 DBMIND_TEST_AGENTS=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        let port: u16 = std::env::var("DBMIND_TEST_ES_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(9212);
        let (_dir, engine, _conn) = fixture("host-layer-parallel-es");
        let es = engine
            .add_connection(
                ConnectionConfig::new("es", ConnectionKind::Elasticsearch)
                    .with_host("127.0.0.1")
                    .with_port(port),
            )
            .expect("添加 Elasticsearch 连接失败");
        let run = |sql: &str| engine.execute(QueryRequest::new(es.id.as_str(), sql), AccessContext::Desktop);

        // 先确认真连得上：替身没起来就该在这里失败并说清怎么办
        run("GET /logs/_search").unwrap_or_else(|e| {
            panic!(
                "连不上 ES 替身（127.0.0.1:{port}）：{} —— 先跑 scripts/host-tests.ps1",
                e.message
            )
        });

        // 顺序调用：**粘性**
        let first = run("GET /logs/_search").expect("查询失败");
        let second = run("GET /logs/_search").expect("查询失败");
        assert_eq!(first.session_id, second.session_id, "顺序调用应当复用同一条会话");

        // 并发：同时发 8 条
        let ids: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8).map(|_| scope.spawn(|| run("GET /logs/_search"))).collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .expect("查询线程不应 panic")
                        .expect("并发查询应当成功")
                        .session_id
                        .unwrap_or_default()
                })
                .collect()
        });

        // 与 Mongo 那条同一套断言：同一泳道、真的开出第二条、不超过类型声明的上限
        let mut lanes = std::collections::BTreeSet::new();
        let mut slots = std::collections::BTreeSet::new();
        for id in &ids {
            let (lane, slot) = id.rsplit_once('#').expect("会话 id 形如 <泳道>#<槽位>");
            lanes.insert(lane.to_string());
            slots.insert(slot.to_string());
        }
        assert_eq!(lanes.len(), 1, "并发请求都在同一条连接（同一个泳道）上：{ids:?}");
        assert!(
            slots.len() >= 2,
            "可多连接类型的并发应当**真的开出第二条**物理会话（惰性生长）：{ids:?}"
        );
        let limit = ConnectionKind::Elasticsearch.max_connections();
        assert!(
            slots.len() <= limit,
            "槽位数不该超过类型声明的上限 {limit}：{ids:?}"
        );
        println!(
            "ES 并发 8 条落在 {} 条会话上（槽位 {slots:?}，类型上限 {limit}）",
            slots.len()
        );
    }

    /// **宿主层**（默认不跑）：**专属协议宿主里的单连接类型**（Redis）永远只用一条会话。
    ///
    /// 补的是覆盖矩阵里那格：④ 的单连接串行用的是 H2（JDBC 宿主），原生侧是 SQLite，
    /// 而 **Redis 是「专属协议宿主 + 单连接类型」**—— 机制（`singleConnectionPool`）
    /// 是内核共用的，但会话管理在各宿主里是独立实现，所以这一格得单独占一次。
    ///
    /// 断言方式与多连接那两条**同形、结论相反**：8 条并发请求**全部**落在同一条会话
    /// （槽位只能是 `#0`）—— 单连接类型下池里只有一把锁，这就是它的直接可观察后果。
    /// 需要 Redis 替身（`deploy/redis-test-server`，`scripts/host-tests.ps1` 会起在 6319）；
    /// 端口可用 `DBMIND_TEST_REDIS_PORT` 覆盖。
    #[test]
    #[ignore = "宿主层：需要真实 JVM + Redis 测试替身（用 scripts/host-tests.ps1）"]
    fn 宿主层_专属单连接类型的并发共用同一条会话_redis() {
        assert!(
            crate::agent::agent_hosts_visible(),
            "这一层要求真实宿主：请设 DBMIND_TEST_AGENTS=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        let port: u16 = std::env::var("DBMIND_TEST_REDIS_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6319);
        let (_dir, engine, _conn) = fixture("host-layer-single-redis");
        let redis = engine
            .add_connection(
                ConnectionConfig::new("redis", ConnectionKind::Redis)
                    .with_host("127.0.0.1")
                    .with_port(port),
            )
            .expect("添加 Redis 连接失败");
        let run =
            |sql: &str| engine.execute(QueryRequest::new(redis.id.as_str(), sql), AccessContext::Desktop);

        // 先确认真连得上：替身没起来就该在这里失败并说清怎么办
        run("PING").unwrap_or_else(|e| {
            panic!(
                "连不上 Redis 替身（127.0.0.1:{port}）：{} —— 先跑 scripts/host-tests.ps1",
                e.message
            )
        });

        // 顺序调用：**粘性**
        let first = run("PING").expect("查询失败");
        let second = run("PING").expect("查询失败");
        assert_eq!(first.session_id, second.session_id, "顺序调用应当复用同一条会话");

        // 并发：同时发 8 条
        let ids: Vec<String> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8).map(|_| scope.spawn(|| run("PING"))).collect();
            handles
                .into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .expect("查询线程不应 panic")
                        .expect("并发查询应当成功")
                        .session_id
                        .unwrap_or_default()
                })
                .collect()
        });

        let mut lanes = std::collections::BTreeSet::new();
        let mut slots = std::collections::BTreeSet::new();
        for id in &ids {
            let (lane, slot) = id.rsplit_once('#').expect("会话 id 形如 <泳道>#<槽位>");
            lanes.insert(lane.to_string());
            slots.insert(slot.to_string());
        }
        assert_eq!(lanes.len(), 1, "并发请求都在同一条连接（同一个泳道）上：{ids:?}");
        // 与多连接那两条正好相反：单连接类型**不可能**开出第二条会话
        assert_eq!(
            slots.len(),
            1,
            "声明了 singleConnectionPool 的类型只允许一条物理会话：{ids:?}"
        );
        assert_eq!(
            slots.iter().next().map(String::as_str),
            Some("0"),
            "而且就是那条唯一的槽位：{ids:?}"
        );
        assert_eq!(
            first.session_id.as_deref(),
            ids.first().map(String::as_str),
            "并发的那些与顺序的必须是同一条会话"
        );
        assert_eq!(
            ConnectionKind::Redis.max_connections(),
            1,
            "这条测试的前提是类型声明：上限为 1"
        );
        println!("Redis 并发 8 条全部落在同一条会话上（槽位 {slots:?}）");
    }

    /// **宿主层**（默认不跑）：专属宿主上的「**排队等会话时也能被取消**」（Redis）。
    ///
    /// ⑥ 用的是 H2（JDBC 宿主），这里换成 **Redis（专属协议宿主 + 单连接类型）**，
    /// 把同一条要求在另一种宿主实现上也钉住。
    ///
    /// 造「慢命令」用的是 `DEBUG SLEEP`（**真实 Redis 也有这条命令**）：宿主并不知道它慢，
    /// 照常按「一次请求一次响应」处理 —— 而这正是能造出排队场景的原因。
    ///
    /// 顺带把 Redis 侧的取消**边界**测出来并钉住：正在跑的命令**不会被中途打断**，
    /// 它的结果在渲染阶段被检查令牌、以「已取消」返回 —— 也就是说**服务端可能已经执行过**。
    /// 这是宿主设计上的取舍（阻断类命令被明确拒绝，见其源码注释），不是漏做。
    #[test]
    #[ignore = "宿主层：需要真实 JVM + Redis 测试替身（用 scripts/host-tests.ps1）"]
    fn 宿主层_专属宿主的排队等待也能被取消_redis() {
        assert!(
            crate::agent::agent_hosts_visible(),
            "这一层要求真实宿主：请设 DBMIND_TEST_AGENTS=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        let port: u16 = std::env::var("DBMIND_TEST_REDIS_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6319);
        let (_dir, engine, _conn) = fixture("host-layer-queued-cancel-redis");
        let redis = engine
            .add_connection(
                ConnectionConfig::new("redis", ConnectionKind::Redis)
                    .with_host("127.0.0.1")
                    .with_port(port),
            )
            .expect("添加 Redis 连接失败");
        let run =
            |sql: &str| engine.execute(QueryRequest::new(redis.id.as_str(), sql), AccessContext::Desktop);
        run("PING").unwrap_or_else(|e| {
            panic!(
                "连不上 Redis 替身（127.0.0.1:{port}）：{} —— 先跑 scripts/host-tests.ps1",
                e.message
            )
        });

        let slow_id = DbMindEngine::next_execution_id();
        let queued_id = DbMindEngine::next_execution_id();
        let (hit, queued, waited, slow_outcome) = std::thread::scope(|scope| {
            // 慢命令占住那唯一一条会话
            let slow_handle = scope.spawn(|| {
                engine.execute(
                    QueryRequest::new(redis.id.as_str(), "DEBUG SLEEP 1").with_execution_id(slow_id.clone()),
                    AccessContext::Desktop,
                )
            });
            std::thread::sleep(std::time::Duration::from_millis(300));

            // 第二条只能排队（单连接类型）
            let queued_handle = scope.spawn(|| {
                engine.execute(
                    QueryRequest::new(redis.id.as_str(), "PING").with_execution_id(queued_id.clone()),
                    AccessContext::Desktop,
                )
            });
            std::thread::sleep(std::time::Duration::from_millis(200));

            let started = std::time::Instant::now();
            let hit = engine.cancel(&queued_id);
            let queued = queued_handle.join().expect("查询线程不应 panic");
            let waited = started.elapsed();

            // 正在跑的那条也取消：它不会被打断，但结果应当以「已取消」返回
            engine.cancel(&slow_id);
            let slow_outcome = slow_handle.join().expect("查询线程不应 panic");
            (hit, queued, waited, slow_outcome)
        });

        assert!(hit, "取消应当命中排队中的那条");
        let err = queued.expect_err("被取消的排队查询不该返回结果");
        assert_eq!(
            err.code,
            ErrorCode::QueryCanceled,
            "排队被取消应报「已取消」，实际：{} {} / {}",
            err.code_str(),
            err.message,
            err.detail.as_deref().unwrap_or("-")
        );
        assert!(
            err.message.contains("等待可用连接"),
            "要说清是**排队时**被取消的：{}",
            err.message
        );
        println!("Redis 排队被取消：{waited:?} 就返回了");
        assert!(
            waited < std::time::Duration::from_secs(2),
            "排队被取消要立刻返回，不能等慢命令跑完：实测 {waited:?}"
        );
        assert_eq!(
            slow_outcome.unwrap_err().code,
            ErrorCode::QueryCanceled,
            "正在跑的 Redis 命令也以「已取消」返回（但服务端可能已经执行过）"
        );

        // 取消之后会话照常可用
        let after = run("PING").expect("取消之后会话必须还能用");
        assert!(
            !after.rows.is_empty() || after.row_count > 0,
            "PING 应当有结果：{after:?}"
        );
    }

    /// **宿主层**（默认不跑）：**可多连接**的专属宿主（Elasticsearch）上的排队取消。
    ///
    /// 与 ⑥（H2 / JDBC 单连接）、⑨（Redis / 单连接）凑成一组：这次要先**占满 4 条会话**
    /// （类型声明的上限），第 5 条才会排队 —— 单连接类型上「排队」是自然发生的，
    /// 多连接类型上得先把池填满，这也是这条测试比它们长一点的原因。
    ///
    /// 慢响应靠替身的 `?slow_ms=` 旋钮（真实集群不会这么慢）。
    /// 顺带钉住 ES 侧的取消边界（宿主源码里写着「已在途的 HTTP 请求无法中断；
    /// 取消影响后续步骤」）：正在跑的请求**不会被打断**，但结果以「已取消」返回。
    #[test]
    #[ignore = "宿主层：需要真实 JVM + Elasticsearch 测试替身（用 scripts/host-tests.ps1）"]
    fn 宿主层_可多连接类型_es_的排队等待也能被取消() {
        assert!(
            crate::agent::agent_hosts_visible(),
            "这一层要求真实宿主：请设 DBMIND_TEST_AGENTS=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        let port: u16 = std::env::var("DBMIND_TEST_ES_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(9212);
        let (_dir, engine, _conn) = fixture("host-layer-queued-cancel-es");
        let es = engine
            .add_connection(
                ConnectionConfig::new("es", ConnectionKind::Elasticsearch)
                    .with_host("127.0.0.1")
                    .with_port(port),
            )
            .expect("添加 Elasticsearch 连接失败");
        let run = |sql: &str| engine.execute(QueryRequest::new(es.id.as_str(), sql), AccessContext::Desktop);
        run("GET /logs/_search").unwrap_or_else(|e| {
            panic!(
                "连不上 ES 替身（127.0.0.1:{port}）：{} —— 先跑 scripts/host-tests.ps1",
                e.message
            )
        });

        // 慢查询：替身会先睡 1.2 秒再回。要够长 —— 必须在**所有 4 条会话都被占住**
        // 的窗口里完成「发第 5 条 + 取消它」这两步（第一次我用了 300ms，太短：
        // 填充查询一结束就有槽位空出来，第 5 条根本没排队 ⇒ 断言抓到了这一点）。
        let slow = "GET /logs/_search?slow_ms=1200";
        let queued_id = DbMindEngine::next_execution_id();
        let filler_ids: Vec<String> = (0..4).map(|_| DbMindEngine::next_execution_id()).collect();

        let (hit, queued, waited, fillers) = std::thread::scope(|scope| {
            // 先把 4 条会话全占住（这个类型声明的上限就是 4）
            let handles: Vec<_> = filler_ids
                .iter()
                .map(|id| {
                    let id = id.clone();
                    // 显式取引用后再 move：否则 `move` 会把 engine / es 搬进循环里的第一个闭包
                    let engine = &engine;
                    let es = &es;
                    scope.spawn(move || {
                        engine.execute(
                            QueryRequest::new(es.id.as_str(), slow).with_execution_id(id),
                            AccessContext::Desktop,
                        )
                    })
                })
                .collect();
            // 槽位是空的 ⇒ 它们是立刻拿到会话的（毫秒级），这条 sleep 只是留足余量
            std::thread::sleep(std::time::Duration::from_millis(300));

            // 第 5 条：4 条会话都在忙，它只能排队
            let queued_handle = scope.spawn(|| {
                engine.execute(
                    QueryRequest::new(es.id.as_str(), "GET /logs/_search")
                        .with_execution_id(queued_id.clone()),
                    AccessContext::Desktop,
                )
            });
            std::thread::sleep(std::time::Duration::from_millis(250));

            let started = std::time::Instant::now();
            let hit = engine.cancel(&queued_id);
            let queued = queued_handle.join().expect("查询线程不应 panic");
            let waited = started.elapsed();

            // 收尾：把 4 条慢的也取消 —— **先把四条都取消，再逐个 join**。
            // 边取消边 join 是错的（我第一次就这么写）：join 会一直等到那条慢查询结束，
            // 于是后面的取消排在前一次等待之后，最后那条早已跑完 ⇒ 「取消未命中」。
            for id in filler_ids.iter() {
                engine.cancel(id);
            }
            let fillers: Vec<_> = handles
                .into_iter()
                .map(|handle| handle.join().expect("查询线程不应 panic"))
                .collect();
            (hit, queued, waited, fillers)
        });

        assert!(hit, "取消应当命中排队中的那条");
        let err = queued.expect_err("被取消的排队查询不该返回结果");
        assert_eq!(
            err.code,
            ErrorCode::QueryCanceled,
            "排队被取消应报「已取消」，实际：{} {} / {}",
            err.code_str(),
            err.message,
            err.detail.as_deref().unwrap_or("-")
        );
        assert!(
            err.message.contains("等待可用连接"),
            "要说清是**排队时**被取消的：{}",
            err.message
        );
        println!("ES 排队被取消：{waited:?} 就返回了（4 条会话都被占住的情况下）");
        assert!(
            waited < std::time::Duration::from_secs(2),
            "排队被取消要立刻返回，不能等 4 条慢查询跑完：实测 {waited:?}"
        );

        // 4 条在跑的都应当以「已取消」返回（ES 宿主无法中断在途 HTTP，见其源码注释）
        for filler in fillers {
            assert_eq!(
                filler.unwrap_err().code,
                ErrorCode::QueryCanceled,
                "在跑的 ES 查询也应当以「已取消」返回"
            );
        }

        // 取消之后会话照常可用
        let after = run("GET /logs/_search").expect("取消之后会话必须还能用");
        assert!(
            after.session_id.as_deref().is_some_and(|s| !s.is_empty()),
            "结果要带上会话 id：{after:?}"
        );
    }

    /// **宿主层**（默认不跑）：`truncated` 在 **agent 侧也如实** —— 与默认层那条
    /// `原生驱动_恰好等于上限时不算截断` 凑成一对（那边验原生，这边验两个协议宿主）。
    ///
    /// 为什么要专门钉住：统一「原生 vs JDBC」时我只确认了 **JDBC** 宿主是对的
    /// （`setMaxRows(maxRows + 1)`）；**ES 与 Mongo 是读代码看出来的、当时没有测试** ——
    /// ES 的 `clip` 是 `rows.size() <= maxRows` 就不动，Mongo 是 `truncated = cursor.id != 0
    /// || batch.size() > limit`，两者都是「恰好等于上限 ⇒ 不算截断」，与内核同义。
    /// 这条测试把那句「我读过代码，应该没问题」变成可执行的断言。
    #[test]
    #[ignore = "宿主层：需要真实 JVM + ES / Mongo 替身（用 scripts/host-tests.ps1）"]
    fn 宿主层_截断标记_es_与_mongo_都如实() {
        assert!(
            crate::agent::agent_hosts_visible(),
            "这一层要求真实宿主：请设 DBMIND_TEST_AGENTS=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        let es_port: u16 = std::env::var("DBMIND_TEST_ES_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(9212);
        let mongo_port: u16 = std::env::var("DBMIND_TEST_MONGO_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(27117);
        let (_dir, engine, _conn) = fixture("host-layer-truncation");
        let es = engine
            .add_connection(
                ConnectionConfig::new("es", ConnectionKind::Elasticsearch)
                    .with_host("127.0.0.1")
                    .with_port(es_port),
            )
            .expect("添加 Elasticsearch 连接失败");
        let mongo = engine
            .add_connection(
                ConnectionConfig::new("mongo", ConnectionKind::Mongodb)
                    .with_host("127.0.0.1")
                    .with_port(mongo_port)
                    .with_database("hostlayer"),
            )
            .expect("添加 Mongo 连接失败");

        let check = |label: &str, connection: &str, sql: &str| {
            let run = |max_rows: usize| {
                let mut request = QueryRequest::new(connection, sql);
                request.options.max_rows = max_rows;
                engine.execute(request, AccessContext::Desktop)
            };

            // 先拿总行数（上限给得足够大）
            let all =
                run(100_000).unwrap_or_else(|e| panic!("{label}：查询失败：{} {}", e.code_str(), e.message));
            let total = all.row_count;
            assert!(
                total >= 2,
                "{label}：替身只有 {total} 行，这条测试会退化成空转（先看替身的种子数据）"
            );
            assert!(!all.truncated, "{label}：上限远大于 {total} 行，不该说截断");

            // 恰好等于行数 ⇒ 结果**完整**
            let exact = run(total).unwrap_or_else(|e| panic!("{label}：查询失败：{}", e.message));
            assert_eq!(exact.row_count, total, "{label}：上限 = 行数时应当返回全部");
            assert!(
                !exact.truncated,
                "{label}：恰好 {total} 行 / 上限 {total} 不算截断（这正是本测试要钉的那条）"
            );
            let notices = format!("{:?}", exact.notices);
            assert!(
                !notices.contains("截断"),
                "{label}：完整结果不该带截断提示：{notices}"
            );

            // 少一行 ⇒ 真的被截断，且只返回上限那么多行
            let cut = run(total - 1).unwrap_or_else(|e| panic!("{label}：查询失败：{}", e.message));
            assert_eq!(cut.row_count, total - 1, "{label}：截断时只该返回上限那么多行");
            assert!(
                cut.truncated,
                "{label}：{total} 行 / 上限 {} 必须说截断",
                total - 1
            );
            println!(
                "{label}：共 {total} 行 —— 上限 {total} ⇒ 不截断；上限 {} ⇒ 截断 ✓",
                total - 1
            );
        };

        check("ES", es.id.as_str(), "GET /logs/_search");
        // Mongo 替身是内嵌的内存版，**默认一个集合都没有** ⇒ 先放三行进去，
        // 这条测试自给自足（不依赖替身的种子数据 —— 我第一次直接 find 了一个空集合，
        // 结果拿到 0 行，「恰好等于上限」那条断言根本无从验起）。
        engine
            .execute(
                QueryRequest::new(
                    mongo.id.as_str(),
                    "db.trunc.insertMany([{\"n\": 1}, {\"n\": 2}, {\"n\": 3}])",
                ),
                AccessContext::Desktop,
            )
            .expect("插入测试数据失败");
        check("Mongo", mongo.id.as_str(), "db.trunc.find({})");
    }

    #[test]
    fn 结构浏览与自检() {
        let (_dir, engine, conn) = fixture("schema");
        engine
            .execute(
                QueryRequest::new(
                    conn.as_str(),
                    "create table t(id integer primary key, name text not null)",
                ),
                AccessContext::Desktop,
            )
            .unwrap();
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "create view v as select id from t"),
                AccessContext::Desktop,
            )
            .unwrap();

        let tables = engine.list_tables(&conn).unwrap();
        let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"t"));
        assert!(names.contains(&"v"));
        assert_eq!(
            tables.iter().find(|t| t.name == "v").unwrap().kind,
            crate::types::TableKind::View
        );

        let columns = engine.list_columns(&conn, "t").unwrap();
        assert_eq!(columns.len(), 2);
        assert!(columns[0].primary_key);
        assert!(!columns[1].nullable);

        let report = engine.test_connection(&conn).unwrap();
        assert!(report.server_version.unwrap().starts_with("SQLite"));
        assert_eq!(report.runtime_mode, RuntimeMode::Native);

        // 自检失败要归到连接错误码
        let err = engine.test_connection("不存在的连接").unwrap_err();
        assert_eq!(err.code, ErrorCode::ConnNotFound);
    }

    #[test]
    fn 成功与失败都会留痕() {
        let (_dir, engine, conn) = fixture("history");
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "select 1"),
                AccessContext::Desktop,
            )
            .unwrap();
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "select * from 不存在的表"),
                AccessContext::Desktop,
            )
            .unwrap_err();

        let history = engine.history(10, None).unwrap();
        assert_eq!(history.len(), 2);
        assert!(history.iter().any(|h| h.status == HistoryStatus::Ok));
        let failed = history.iter().find(|h| h.status == HistoryStatus::Error).unwrap();
        assert_eq!(failed.error_code.as_deref(), Some("DBMIND-QUERY-0002"));

        // 按连接过滤
        assert_eq!(engine.history(10, Some(conn.as_str())).unwrap().len(), 2);
        assert_eq!(engine.clear_history().unwrap(), 2);
    }

    /// `internal=true` 的调用**不进历史**；失败的内部调用同样不进。
    ///
    /// 这条钉的是首页「最近查询 / 查询统计」：实机上它被元数据查询淹了 ——
    /// `show databases`、取表选项的 `select table_name …` 全在里面，反而看不到用户敲的 SQL。
    /// 判据必须是**调用方显式声明**：这些内部语句与用户 SQL 在文本上无从区分。
    #[test]
    fn 内部调用不进历史() {
        let (_dir, engine, conn) = fixture("history-internal");
        // 元数据 / 结构浏览类：用户看不出来是它干的，不进历史
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "select name from sqlite_master").with_internal(true),
                AccessContext::Web,
            )
            .unwrap();
        // 内部调用失败同样不留痕（失败也不该冒到「最近查询」里）
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "select * from 不存在的表").with_internal(true),
                AccessContext::Web,
            )
            .unwrap_err();
        assert!(
            engine.history(10, None).unwrap().is_empty(),
            "internal 的调用不该留痕"
        );

        // 用户自己执行的照旧留痕（编辑器那条路走的就是默认值）
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "select 1"),
                AccessContext::Web,
            )
            .unwrap();
        let history = engine.history(10, None).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].sql, "select 1");

        // 会话键前缀那条老口径仍然有效（datagen / sync / import 用的是它）
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "select 2").with_session("internal:browse"),
                AccessContext::Web,
            )
            .unwrap();
        assert_eq!(engine.history(10, None).unwrap().len(), 1, "会话键前缀也应拦住");
    }

    #[test]
    fn 类型目录包含声明与实现状态() {
        let (_dir, engine, _conn) = fixture("types");
        let types = engine.types();
        assert!(types.len() >= 16, "应覆盖上游支持的类型集合");
        let sqlite = types.iter().find(|t| t.key == "sqlite").unwrap();
        assert!(sqlite.implemented);
        assert_eq!(sqlite.runtime_mode, RuntimeMode::Native);
        // JDBC 类型现在由 agent 宿主承载：声明即接入（驱动是否已装是运行期事实）
        let pg = types.iter().find(|t| t.key == "postgresql").unwrap();
        assert!(pg.implemented);
        assert_eq!(pg.runtime_mode, RuntimeMode::Agent);
        assert_eq!(pg.default_port, Some(5432));
        assert!(pg.capabilities.query_execution);
        assert!(pg.kind.is_jdbc(), "PostgreSQL 应有 jdbc 元数据");

        // MongoDB：专属协议宿主已接入，且协议不是 SQL
        let mongo = types.iter().find(|t| t.key == "mongodb").unwrap();
        assert_eq!(mongo.runtime_mode, RuntimeMode::Agent);
        assert!(mongo.implemented);
        assert_eq!(mongo.protocol, crate::RuntimeProtocol::Mongodb);

        // Redis / Elasticsearch：专属宿主也已接入，各自有独立的协议判定
        let redis = types.iter().find(|t| t.key == "redis").unwrap();
        assert_eq!(redis.runtime_mode, RuntimeMode::Agent);
        assert!(redis.implemented);
        assert_eq!(redis.protocol, crate::RuntimeProtocol::Redis);
        assert_eq!(redis.default_port, Some(6379));

        let es = types.iter().find(|t| t.key == "elasticsearch").unwrap();
        assert!(es.implemented);
        assert_eq!(es.protocol, crate::RuntimeProtocol::Elasticsearch);
        assert_eq!(es.default_port, Some(9200));

        // 全类型都不再是「未接入」：这是 17/17 的判定点
        assert!(
            types.iter().all(|t| t.implemented),
            "所有声明都应有执行路径：{:?}",
            types
                .iter()
                .filter(|t| !t.implemented)
                .map(|t| t.key)
                .collect::<Vec<_>>()
        );

        // 文件型引擎也由 agent 承载（DuckDB 官方 JDBC 驱动可在通用宿主里内嵌运行）
        let duck = types.iter().find(|t| t.key == "duckdb").unwrap();
        assert_eq!(duck.runtime_mode, RuntimeMode::Agent);
        assert!(duck.implemented);
        assert!(duck.kind.is_jdbc());
        assert!(duck.local_file, "DuckDB 是文件型连接");

        let summary = engine.runtime_summary();
        assert_eq!(summary.declared_types, types.len());
        // SQLite + 13 个 JDBC 类型（12 个关系型 + DuckDB）
        assert!(
            summary.implemented_types >= 14,
            "已接入类型数应包含 JDBC agent 承载的类型，实际 {}",
            summary.implemented_types
        );
    }

    #[test]
    fn 执行_id_可用于取消登记() {
        let (_dir, engine, conn) = fixture("cancel");
        let id = DbMindEngine::next_execution_id();
        let request = QueryRequest::new(conn.as_str(), "select 1").with_execution_id(id.clone());
        engine.execute(request, AccessContext::Desktop).unwrap();
        // 已结束的执行：取消不应命中
        assert!(!engine.cancel(&id));
        assert!(engine.active_executions().is_empty());
    }

    /// 原生单连接类型（SQLite）上，**排队等连接**的语句也能被立刻取消。
    ///
    /// 同一文件只有一个连接对象：第一条慢语句占着它时，第二条会阻塞在连接锁上。
    /// 要求与 agent 侧会话池一样 —— 点「取消」立刻有反应，而不是等前一条跑完。
    /// 这条不需要 JVM，所以留在**默认层**（跑得快，能天天跑）。
    #[test]
    fn 原生连接上排队等锁时也能被取消() {
        let (_dir, engine, conn) = fixture("native-queued-cancel");
        let run = |sql: &str| engine.execute(QueryRequest::new(conn.as_str(), sql), AccessContext::Desktop);
        run("create table t(id integer primary key)").expect("建表应当成功");

        // 慢语句要**逐行产出**（原生驱动在行流上检查取消），且每行真的慢
        //（`randomblob(4000000)` 本机约 73ms/行 ⇒ 默认 2000 行上限也要跑很久）
        let slow = "with recursive c(x) as (select 1 union all select x+1 from c where x < 100000) \
                    select length(randomblob(4000000)) as n from c";
        let slow_id = DbMindEngine::next_execution_id();
        let queued_id = DbMindEngine::next_execution_id();

        let (hit, queued, waited, slow_outcome) = std::thread::scope(|scope| {
            let slow_handle = scope.spawn(|| {
                engine.execute(
                    QueryRequest::new(conn.as_str(), slow).with_execution_id(slow_id.clone()),
                    AccessContext::Desktop,
                )
            });
            // 让它真的拿到那条连接
            std::thread::sleep(std::time::Duration::from_millis(300));

            let queued_handle = scope.spawn(|| {
                engine.execute(
                    QueryRequest::new(conn.as_str(), "select 1 as x").with_execution_id(queued_id.clone()),
                    AccessContext::Desktop,
                )
            });
            // 让它走到连接锁的等待里
            std::thread::sleep(std::time::Duration::from_millis(300));

            let started = std::time::Instant::now();
            let hit = engine.cancel(&queued_id);
            let queued = queued_handle.join().expect("查询线程不应 panic");
            let waited = started.elapsed();

            // 收尾：把慢的那条也取消（测试才不会跑很久）
            engine.cancel(&slow_id);
            let slow_outcome = slow_handle.join().expect("查询线程不应 panic");
            (hit, queued, waited, slow_outcome)
        });

        assert!(hit, "取消应当命中排队中的那条");
        let err = queued.expect_err("被取消的排队查询不该返回结果");
        assert_eq!(
            err.code,
            ErrorCode::QueryCanceled,
            "排队被取消应报「已取消」，实际：{} {}",
            err.code_str(),
            err.message
        );
        assert!(
            err.message.contains("等待可用连接"),
            "要说清是**排队时**被取消的（与跑到一半被取消区分开）：{}",
            err.message
        );
        println!("原生连接上排队被取消：{waited:?} 就返回了");
        assert!(
            waited < std::time::Duration::from_secs(3),
            "排队被取消要立刻返回，不能等前一条跑完：实测 {waited:?}"
        );
        assert_eq!(
            slow_outcome.unwrap_err().code,
            ErrorCode::QueryCanceled,
            "慢语句也要能被取消（收尾靠它）"
        );

        // 取消只掐语句，不该把连接弄坏
        let after = run("select count(*) from t").expect("取消之后连接必须还能用");
        assert_eq!(after.rows[0][0], CellValue::Integer(0));
    }

    /// 请求超过**硬上限**时按上限截断 —— **端到端**，不只是类型层。
    ///
    /// `QueryOptions::HARD_MAX_ROWS = 100_000` 且内核每请求都 `clamp()`（`engine::execute`）：
    /// 壳层可以传任意大，真正生效的是内核这一关。类型层已有单测（`QueryOptions::clamp`），
    /// 这里补的是**真的走到驱动**那一段：请求 `usize::MAX` ⇒ 实际只取 10 万行，
    /// 而且截断提示里说的是**生效的上限**（不是用户传的那个数）。
    #[test]
    fn 请求超过硬上限时按上限截断() {
        let (_dir, engine, conn) = fixture("hard-max-rows");
        let mut request = QueryRequest::new(
            conn.as_str(),
            "with recursive c(x) as (select 1 union all select x+1 from c where x < 200000) \
             select x from c",
        );
        request.options.max_rows = usize::MAX; // 壳层能传多大就传多大
        let result = engine
            .execute(request, AccessContext::Desktop)
            .expect("查询应当成功");

        assert_eq!(QueryOptions::HARD_MAX_ROWS, 100_000, "这条测试的前提");
        assert!(result.truncated, "超过上限应当置截断标记");
        assert_eq!(
            result.row_count,
            QueryOptions::HARD_MAX_ROWS,
            "只应当取到上限那么多行（多一行都不给）"
        );
        assert!(
            result
                .notices
                .iter()
                .any(|notice| notice.contains(&QueryOptions::HARD_MAX_ROWS.to_string())),
            "截断提示要说出生效的上限：{:?}",
            result.notices
        );
    }

    /// 截断标记必须**如实**：结果恰好等于上限时不该说「已截断」。
    ///
    /// 这是两个驱动曾经不一致的地方：JDBC 宿主靠 `setMaxRows(maxRows + 1)` 从结果集
    /// 那一头多取一行，所以它的判定是准的；原生驱动则是「取满上限就置位」，于是
    /// 「结果恰好等于上限」也被标成截断 —— 界面上就会平白多一句「结果已截断」。
    /// 现在两边同义：**多取一行来看，才敢说被截断**。
    #[test]
    fn 原生驱动_恰好等于上限时不算截断() {
        let (_dir, engine, conn) = fixture("truncated-exact");
        let run = |sql: &str, max_rows: usize| {
            let mut request = QueryRequest::new(conn.as_str(), sql);
            request.options.max_rows = max_rows;
            engine.execute(request, AccessContext::Desktop)
        };

        // 恰好 10 行、上限 10 ⇒ 结果**完整**，不该说截断
        let exact = run(
            "with recursive c(x) as (select 1 union all select x+1 from c where x < 10) \
             select x from c",
            10,
        )
        .expect("查询应当成功");
        assert_eq!(exact.row_count, 10);
        assert!(
            !exact.truncated,
            "刚好取满上限不算截断（多取一行才敢这么说）：{:?}",
            exact.notices
        );

        // 11 行、上限 10 ⇒ 真的被截断，且只返回上限那么多行
        let more = run(
            "with recursive c(x) as (select 1 union all select x+1 from c where x < 11) \
             select x from c",
            10,
        )
        .expect("查询应当成功");
        assert_eq!(more.row_count, 10, "只返回上限那么多行");
        assert!(more.truncated, "真有第 11 行 ⇒ 必须标截断");
    }

    #[test]
    fn 被拦截与失败的尝试同样留痕() {
        let (_dir, engine, conn) = fixture("audit");
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "create table t(a integer)"),
                AccessContext::Desktop,
            )
            .unwrap();
        engine.set_read_only(&conn, true).unwrap();

        // 安全策略拦截的写操作
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "drop table t"),
                AccessContext::Desktop,
            )
            .unwrap_err();
        // 多条语句
        engine
            .execute(
                QueryRequest::new(conn.as_str(), "select 1; select 2"),
                AccessContext::Desktop,
            )
            .unwrap_err();
        // 连接本身就不存在
        engine
            .execute(QueryRequest::new("ghost", "select 1"), AccessContext::Desktop)
            .unwrap_err();

        let history = engine.history(20, None).unwrap();
        let codes: Vec<&str> = history.iter().filter_map(|h| h.error_code.as_deref()).collect();
        assert!(
            codes.contains(&"DBMIND-SAFETY-0001"),
            "被拦截的写操作应留痕：{codes:?}"
        );
        assert!(codes.contains(&"DBMIND-QUERY-0001"), "非法 SQL 应留痕：{codes:?}");
        assert!(
            codes.contains(&"DBMIND-CONN-0001"),
            "访问不存在的连接应留痕：{codes:?}"
        );

        // 不存在的连接没有 connectionId，但要保留用户输入的名称，便于追责
        let dangling = history
            .iter()
            .find(|h| h.error_code.as_deref() == Some("DBMIND-CONN-0001"))
            .unwrap();
        assert_eq!(dangling.connection_name.as_deref(), Some("ghost"));
        assert!(dangling.connection_id.is_none());
    }

    /// **宿主层**（默认不跑）：连接被**服务端切断**之后，这条连接仍然能自愈。
    ///
    /// 这条钉的是一个**缺陷类**（真机 Redis 验证抓到的）：连接坏掉之后，如果宿主不自愈、
    /// 内核又不清自己的会话缓存，用户这条连接会**一直坏下去** —— 直到他手动重连。
    /// 关键前提：内核**不重试**非幂等查询（`retry_on_stale` 只对幂等调用为真），
    /// 所以「重试」不是答案，「下一次调用能重新连上」才是。
    ///
    /// 造法用 Redis 的 `QUIT`：真 Redis 与替身都是「回 `+OK` 然后关掉连接」。于是——
    /// ① 紧接着那条命令如实报「连接已断开」（**这是预期的**，不是缺陷）；
    /// ② **再下一条必须自己好起来**（符合任意一层的修复：宿主丢会话，或内核清缓存）。
    #[test]
    #[ignore = "宿主层：需要真实 JVM + Redis 测试替身（用 scripts/host-tests.ps1）"]
    fn 宿主层_断链之后仍能自愈() {
        assert!(
            crate::agent::agent_hosts_visible(),
            "这一层要求真实宿主：请设 DBMIND_TEST_AGENTS=1 后重跑（或直接用 scripts/host-tests.ps1）"
        );
        let port: u16 = std::env::var("DBMIND_TEST_REDIS_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6319);
        let (_dir, engine, _conn) = fixture("host-layer-broken-connection");
        let redis = engine
            .add_connection(
                ConnectionConfig::new("redis", ConnectionKind::Redis)
                    .with_host("127.0.0.1")
                    .with_port(port),
            )
            .expect("添加 Redis 连接失败");
        let run =
            |sql: &str| engine.execute(QueryRequest::new(redis.id.as_str(), sql), AccessContext::Desktop);

        run("PING").unwrap_or_else(|e| {
            panic!(
                "连不上 Redis 替身（127.0.0.1:{port}）：{} —— 先跑 scripts/host-tests.ps1",
                e.message
            )
        });

        // 让**服务端**把这条连接关掉
        let quit = run("QUIT").expect("QUIT 应当正常返回");
        assert!(
            format!("{:?}", quit.rows).contains("OK"),
            "QUIT 应当回 OK：{:?}",
            quit.rows
        );

        // ① 紧接着那条：如实报「连接已断开」—— 这是切断的必然结果，不是缺陷
        let err = run("PING").expect_err("连接刚被切断，这一条应当失败");
        assert_eq!(
            err.code,
            ErrorCode::ConnConnectFailed,
            "断链应当报连接级错误，实际：{} {} / {}",
            err.code_str(),
            err.message,
            err.detail.as_deref().unwrap_or("-")
        );
        println!("断链之后第一条如实报错：{} {}", err.code_str(), err.message);

        // ② **再下一条必须自己好起来** —— 这才是这条测试真正钉住的东西
        let after = run("PING").unwrap_or_else(|e| {
            panic!(
                "断链之后应当自愈（宿主丢会话 / 内核清缓存），实际：{} {}",
                e.code_str(),
                e.message
            )
        });
        assert!(
            format!("{:?}", after.rows).contains("PONG"),
            "自愈之后 PING 应当回 PONG：{:?}",
            after.rows
        );
        println!("断链之后会话自愈 ✓");
    }
}
