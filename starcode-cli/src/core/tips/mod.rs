/// Tips提示系统
/// 
/// 对标claude-code-main的src/services/tips/
/// 提供使用技巧和最佳实践提示

use serde::{Deserialize, Serialize};

/// 提示类型
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TipType {
    /// 快捷键
    Shortcut,
    /// 最佳实践
    BestPractice,
    /// 功能介绍
    Feature,
    /// 性能优化
    Performance,
    /// 安全提示
    Security,
}

/// 提示
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tip {
    /// 提示ID
    pub id: String,
    /// 提示类型
    pub tip_type: TipType,
    /// 标题
    pub title: String,
    /// 内容
    pub content: String,
    /// 相关命令
    pub command: Option<String>,
    /// 优先级
    pub priority: u8,
}

/// 提示管理器
pub struct TipManager {
    tips: Vec<Tip>,
    shown_tips: std::collections::HashSet<String>,
}

impl TipManager {
    pub fn new() -> Self {
        let mut manager = Self {
            tips: Vec::new(),
            shown_tips: std::collections::HashSet::new(),
        };
        
        manager.load_default_tips();
        manager
    }

    /// 加载默认提示
    fn load_default_tips(&mut self) {
        self.tips.push(Tip {
            id: "ctrl_p".to_string(),
            tip_type: TipType::Shortcut,
            title: "Command Palette".to_string(),
            content: "Press Ctrl+P to open the command palette for quick access to commands.".to_string(),
            command: Some("Ctrl+P".to_string()),
            priority: 10,
        });

        self.tips.push(Tip {
            id: "ctrl_c".to_string(),
            tip_type: TipType::Shortcut,
            title: "Cancel".to_string(),
            content: "Press Ctrl+C to cancel the current operation.".to_string(),
            command: Some("Ctrl+C".to_string()),
            priority: 9,
        });

        self.tips.push(Tip {
            id: "memory".to_string(),
            tip_type: TipType::BestPractice,
            title: "Use Memory".to_string(),
            content: "Use the /memory command to save important information for future reference.".to_string(),
            command: Some("/memory".to_string()),
            priority: 8,
        });

        self.tips.push(Tip {
            id: "compact".to_string(),
            tip_type: TipType::Performance,
            title: "Context Compaction".to_string(),
            content: "Use /compact to reduce context size when conversations get long.".to_string(),
            command: Some("/compact".to_string()),
            priority: 7,
        });

        self.tips.push(Tip {
            id: "plan_mode".to_string(),
            tip_type: TipType::Feature,
            title: "Plan Mode".to_string(),
            content: "Use plan mode to review changes before they are applied.".to_string(),
            command: None,
            priority: 6,
        });
    }

    /// 获取随机提示
    pub fn get_random_tip(&mut self) -> Option<&Tip> {
        let unshown: Vec<&Tip> = self.tips.iter()
            .filter(|t| !self.shown_tips.contains(&t.id))
            .collect();

        if unshown.is_empty() {
            // 重置已显示的提示
            self.shown_tips.clear();
            return self.tips.first();
        }

        let index = (chrono::Utc::now().timestamp() as usize) % unshown.len();
        let tip = unshown[index];
        self.shown_tips.insert(tip.id.clone());
        Some(tip)
    }

    /// 按类型获取提示
    pub fn get_tips_by_type(&self, tip_type: &TipType) -> Vec<&Tip> {
        self.tips.iter()
            .filter(|t| std::mem::discriminant(&t.tip_type) == std::mem::discriminant(tip_type))
            .collect()
    }

    /// 获取所有提示
    pub fn get_all_tips(&self) -> &[Tip] {
        &self.tips
    }
}
