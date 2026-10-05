//! 取消令牌与运行中查询登记表。
//!
//! 取消是 shell 无关的能力：CLI 的 Ctrl+C、Web 的 `/cancel`、MCP 的取消通知
//! 都走同一个注册表，避免每个壳各写一套。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
pub struct CancelToken {
    flag: Arc<AtomicBool>,
}

impl CancelToken {
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// 以 executionId 为键的运行中查询登记表。
#[derive(Default)]
pub struct CancelRegistry {
    inner: Mutex<HashMap<String, CancelToken>>,
}

impl CancelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, CancelToken>> {
        // 中毒的锁不影响业务：登记表只是旁路信息，取回内部值继续用
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 登记一次执行并拿到令牌。
    pub fn register(&self, execution_id: &str) -> CancelToken {
        let token = CancelToken::new();
        self.lock().insert(execution_id.to_string(), token.clone());
        token
    }

    /// 执行结束（成功/失败/取消）后必须调用，否则登记表会泄漏。
    pub fn finish(&self, execution_id: &str) {
        self.lock().remove(execution_id);
    }

    /// 请求取消；返回是否命中（false = 该执行已结束或不存在）。
    pub fn cancel(&self, execution_id: &str) -> bool {
        match self.lock().get(execution_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    pub fn active(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.lock().keys().cloned().collect();
        ids.sort();
        ids
    }

    pub fn is_active(&self, execution_id: &str) -> bool {
        self.lock().contains_key(execution_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_cancel_finish() {
        let reg = CancelRegistry::new();
        let token = reg.register("e1");
        assert!(!token.is_cancelled());
        assert!(reg.is_active("e1"));
        assert_eq!(reg.active(), vec!["e1".to_string()]);

        assert!(reg.cancel("e1"));
        assert!(
            token.is_cancelled(),
            "取消后令牌应立刻可见（clone 共享同一 AtomicBool）"
        );

        reg.finish("e1");
        assert!(!reg.is_active("e1"));
        assert!(!reg.cancel("e1"), "已结束的执行不应被取消命中");
    }
}
