//! Virtual-scrolling list with dirty-item tracking.
//!
//! Inspired by tuie's virtualized lists, this component manages:
//! - Per-item height cache
//! - Viewport-aware clipping (only visible items are rendered)
//! - Scroll-anchor preservation (items don't jump when heights change)
//! - Dirty-item tracking (only changed items re-render)
//! - Auto-follow mode (sticks to bottom on new content)

/// Height tracker for a single list item.
#[derive(Debug, Clone, Copy, Default)]
struct ItemState {
    height: u16,
    dirty: bool,
}

/// A virtual-scrolling list that efficiently renders only visible items.
#[derive(Debug, Clone)]
pub struct VirtualList {
    heights: Vec<ItemState>,
    total_lines: usize,
    /// When true, the view automatically follows new items (scroll to bottom).
    pub auto_follow: bool,
}

impl VirtualList {
    pub fn new() -> Self {
        Self {
            heights: Vec::new(),
            total_lines: 0,
            auto_follow: true,
        }
    }

    pub fn len(&self) -> usize { self.heights.len() }

    /// Resize to match the given number of items.
    pub fn resize(&mut self, new_len: usize) {
        self.heights.resize_with(new_len, ItemState::default);
        self.recalc_total();
    }

    /// Mark an item as dirty (needs re-render).
    pub fn mark_dirty(&mut self, index: usize) {
        if let Some(item) = self.heights.get_mut(index) {
            item.dirty = true;
        }
    }

    /// 在 idx 处插入一个新项（中插条目时使用），后续项顺移；新项标记为 dirty。
    pub fn insert_at(&mut self, idx: usize) {
        let idx = idx.min(self.heights.len());
        self.heights.insert(idx, ItemState { height: 0, dirty: true });
        self.recalc_total();
    }

    /// Check if an item is dirty.
    pub fn is_dirty(&self, index: usize) -> bool {
        self.heights.get(index).map(|i| i.dirty).unwrap_or(false)
    }

    /// Clear dirty flag for an item.
    pub fn clear_dirty(&mut self, index: usize) {
        if let Some(item) = self.heights.get_mut(index) {
            item.dirty = false;
        }
    }

    /// Update height for an item and recalculate total.
    pub fn set_height(&mut self, index: usize, height: u16) {
        if let Some(item) = self.heights.get_mut(index) {
            if item.height != height {
                item.height = height;
                self.recalc_total();
            }
        }
    }

    /// Get height for an item.
    pub fn get_height(&self, index: usize) -> u16 {
        self.heights.get(index).map(|i| i.height).unwrap_or(0)
    }

    /// Get total lines across all items.
    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    /// Calculate visible range for a given viewport.
    /// Returns (start_index, end_index, offset_within_start)
    pub fn visible_range(&self, viewport_height: u16, scroll_offset: usize) -> (usize, usize, u16) {
        if self.heights.is_empty() {
            return (0, 0, 0);
        }

        let mut current_line = 0;
        let mut start_index = 0;
        let mut offset_within_start = 0;

        // Find start index
        for (i, item) in self.heights.iter().enumerate() {
            if current_line + item.height as usize > scroll_offset {
                start_index = i;
                offset_within_start = (scroll_offset - current_line) as u16;
                break;
            }
            current_line += item.height as usize;
            if i == self.heights.len() - 1 {
                // Scroll offset exceeds total lines
                return (self.heights.len(), self.heights.len(), 0);
            }
        }

        // Find end index
        let mut visible_lines = 0;
        let mut end_index = start_index;
        for i in start_index..self.heights.len() {
            let item_height = self.heights[i].height;
            let effective_height = if i == start_index {
                item_height - offset_within_start
            } else {
                item_height
            };
            
            if visible_lines + effective_height as usize > viewport_height as usize {
                end_index = i;
                break;
            }
            visible_lines += effective_height as usize;
            end_index = i + 1;
        }

        (start_index, end_index, offset_within_start)
    }

    /// Recalculate total lines.
    fn recalc_total(&mut self) {
        self.total_lines = self.heights.iter().map(|i| i.height as usize).sum();
    }

    /// Mark all items as dirty.
    pub fn mark_all_dirty(&mut self) {
        for item in &mut self.heights {
            item.dirty = true;
        }
    }

    /// Get item height (alias for get_height).
    pub fn item_height(&self, index: usize) -> u16 {
        self.get_height(index)
    }

    /// Get the item index at a given scroll position.
    pub fn item_at_scroll(&self, scroll: usize) -> Option<usize> {
        let mut current = 0;
        for (i, item) in self.heights.iter().enumerate() {
            if current + item.height as usize > scroll {
                return Some(i);
            }
            current += item.height as usize;
        }
        if self.heights.is_empty() {
            None
        } else {
            Some(self.heights.len() - 1)
        }
    }

    /// Calculate scroll offset to keep the given item anchored.
    pub fn anchor_scroll(&self, index: usize, anchor_offset: usize) -> usize {
        let mut scroll = 0;
        for i in 0..index.min(self.heights.len()) {
            scroll += self.heights[i].height as usize;
        }
        scroll.saturating_sub(anchor_offset)
    }
}

/// 虚拟消息列表组件
/// 
/// 对标claude-code-main的VirtualMessageList组件
/// 优化大量消息时的渲染性能

use std::collections::VecDeque;

/// 消息项
#[derive(Debug, Clone)]
pub struct MessageItem {
    /// 消息ID
    pub id: String,
    /// 消息内容
    pub content: String,
    /// 消息类型
    pub message_type: MessageType,
    /// 高度（行数）
    pub height: u16,
    /// 是否可见
    pub visible: bool,
}

/// 消息类型
#[derive(Debug, Clone, PartialEq)]
pub enum MessageType {
    User,
    Assistant,
    System,
    Tool,
    Error,
}

/// 虚拟消息列表
pub struct VirtualMessageList {
    /// 所有消息
    messages: VecDeque<MessageItem>,
    /// 可见区域高度
    viewport_height: u16,
    /// 滚动偏移
    scroll_offset: u16,
    /// 最大消息数
    max_messages: usize,
    /// 自动滚动到底部
    auto_scroll: bool,
}

impl VirtualMessageList {
    /// 创建新的虚拟消息列表
    pub fn new(viewport_height: u16) -> Self {
        Self {
            messages: VecDeque::new(),
            viewport_height,
            scroll_offset: 0,
            max_messages: 1000,
            auto_scroll: true,
        }
    }

    /// 添加消息
    pub fn push(&mut self, message: MessageItem) {
        // 限制消息数量
        if self.messages.len() >= self.max_messages {
            self.messages.pop_front();
        }

        let should_scroll = self.auto_scroll || self.is_at_bottom();
        self.messages.push_back(message);

        if should_scroll {
            self.scroll_to_bottom();
        }
    }

    /// 清空消息
    pub fn clear(&mut self) {
        self.messages.clear();
        self.scroll_offset = 0;
    }

    /// 获取可见消息
    pub fn visible_messages(&self) -> Vec<&MessageItem> {
        let total_height = self.total_height();
        let start = self.scroll_offset;
        let end = (start + self.viewport_height).min(total_height);

        let mut visible = Vec::new();
        let mut current_height = 0;

        for msg in &self.messages {
            if !msg.visible {
                continue;
            }

            let msg_start = current_height;
            let msg_end = current_height + msg.height;

            // 检查消息是否在可见区域内
            if msg_end > start && msg_start < end {
                visible.push(msg);
            }

            current_height = msg_end;

            if current_height >= end {
                break;
            }
        }

        visible
    }

    /// 滚动到底部
    pub fn scroll_to_bottom(&mut self) {
        let total = self.total_height();
        self.scroll_offset = if total > self.viewport_height {
            total - self.viewport_height
        } else {
            0
        };
    }

    /// 滚动到顶部
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
    }

    /// 向上滚动
    pub fn scroll_up(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    /// 向下滚动
    pub fn scroll_down(&mut self, lines: u16) {
        let total = self.total_height();
        let max_offset = if total > self.viewport_height {
            total - self.viewport_height
        } else {
            0
        };
        self.scroll_offset = (self.scroll_offset + lines).min(max_offset);
    }

    /// 是否在底部
    pub fn is_at_bottom(&self) -> bool {
        let total = self.total_height();
        if total <= self.viewport_height {
            return true;
        }
        self.scroll_offset >= total - self.viewport_height
    }

    /// 是否在顶部
    pub fn is_at_top(&self) -> bool {
        self.scroll_offset == 0
    }

    /// 总高度
    pub fn total_height(&self) -> u16 {
        self.messages
            .iter()
            .filter(|m| m.visible)
            .map(|m| m.height)
            .sum()
    }

    /// 消息数量
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    /// 设置视口高度
    pub fn set_viewport_height(&mut self, height: u16) {
        self.viewport_height = height;
        if self.auto_scroll {
            self.scroll_to_bottom();
        }
    }

    /// 设置自动滚动
    pub fn set_auto_scroll(&mut self, auto: bool) {
        self.auto_scroll = auto;
    }

    /// 获取滚动百分比
    pub fn scroll_percentage(&self) -> f64 {
        let total = self.total_height();
        if total <= self.viewport_height {
            return 100.0;
        }
        (self.scroll_offset as f64 / (total - self.viewport_height) as f64) * 100.0
    }

    /// 获取消息引用
    pub fn get_message(&self, id: &str) -> Option<&MessageItem> {
        self.messages.iter().find(|m| m.id == id)
    }

    /// 更新消息
    pub fn update_message(&mut self, id: &str, content: String) -> bool {
        if let Some(msg) = self.messages.iter_mut().find(|m| m.id == id) {
            msg.content = content;
            true
        } else {
            false
        }
    }

    /// 删除消息
    pub fn remove_message(&mut self, id: &str) -> bool {
        if let Some(pos) = self.messages.iter().position(|m| m.id == id) {
            self.messages.remove(pos);
            true
        } else {
            false
        }
    }

    /// 隐藏消息
    pub fn hide_message(&mut self, id: &str) -> bool {
        if let Some(msg) = self.messages.iter_mut().find(|m| m.id == id) {
            msg.visible = false;
            true
        } else {
            false
        }
    }

    /// 显示消息
    pub fn show_message(&mut self, id: &str) -> bool {
        if let Some(msg) = self.messages.iter_mut().find(|m| m.id == id) {
            msg.visible = true;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_message(id: &str, content: &str) -> MessageItem {
        MessageItem {
            id: id.to_string(),
            content: content.to_string(),
            message_type: MessageType::User,
            height: 1,
            visible: true,
        }
    }

    #[test]
    fn test_push_and_len() {
        let mut list = VirtualMessageList::new(10);
        assert!(list.is_empty());

        list.push(create_test_message("1", "Hello"));
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn test_max_messages() {
        let mut list = VirtualMessageList::new(10);
        list.max_messages = 3;

        list.push(create_test_message("1", "A"));
        list.push(create_test_message("2", "B"));
        list.push(create_test_message("3", "C"));
        list.push(create_test_message("4", "D"));

        assert_eq!(list.len(), 3);
        assert!(list.get_message("1").is_none());
        assert!(list.get_message("4").is_some());
    }

    #[test]
    fn test_scroll() {
        let mut list = VirtualMessageList::new(5);
        for i in 0..20 {
            list.push(create_test_message(&i.to_string(), &format!("Message {}", i)));
        }

        assert!(list.is_at_bottom());
        list.scroll_to_top();
        assert!(list.is_at_top());
        assert!(!list.is_at_bottom());
    }

    #[test]
    fn test_update_message() {
        let mut list = VirtualMessageList::new(10);
        list.push(create_test_message("1", "Hello"));

        assert!(list.update_message("1", "World".to_string()));
        assert_eq!(list.get_message("1").unwrap().content, "World");
    }

    #[test]
    fn test_virtual_list() {
        let mut list = VirtualList::new();
        list.resize(10);
        
        assert_eq!(list.len(), 10);
        
        list.set_height(0, 5);
        list.set_height(1, 3);
        
        assert_eq!(list.get_height(0), 5);
        assert_eq!(list.get_height(1), 3);
        assert_eq!(list.total_lines(), 8);
        
        list.mark_dirty(0);
        assert!(list.is_dirty(0));
        
        list.clear_dirty(0);
        assert!(!list.is_dirty(0));
    }
}
