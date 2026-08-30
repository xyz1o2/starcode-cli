/// 余额追踪器

use std::collections::HashMap;

/// 余额追踪器
pub struct BalanceTracker {
    /// 余额映射
    balances: HashMap<String, f64>,
}

impl BalanceTracker {
    /// 创建新的余额追踪器
    pub fn new() -> Self {
        Self {
            balances: HashMap::new(),
        }
    }

    /// 获取余额
    pub fn get_balance(&self, provider: &str) -> Option<f64> {
        self.balances.get(provider).copied()
    }

    /// 更新余额
    pub fn update_balance(&mut self, provider: &str, balance: f64) {
        self.balances.insert(provider.to_string(), balance);
    }

    /// 扣除余额
    pub fn deduct_balance(&mut self, provider: &str, amount: f64) -> bool {
        if let Some(balance) = self.balances.get_mut(provider) {
            if *balance >= amount {
                *balance -= amount;
                return true;
            }
        }
        false
    }

    /// 获取所有余额
    pub fn get_all_balances(&self) -> &HashMap<String, f64> {
        &self.balances
    }
}
