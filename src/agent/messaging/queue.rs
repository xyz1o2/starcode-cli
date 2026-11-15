use parking_lot::Mutex;
use std::collections::VecDeque;

/// 使用短临界区维护 FIFO；取走与撤回必须在同一把锁内决定归属。
pub struct AsyncMessageQueue<T> {
    items: Mutex<VecDeque<T>>,
}

impl<T: Send + 'static> AsyncMessageQueue<T> {
    pub fn new() -> Self {
        Self {
            items: Mutex::new(VecDeque::new()),
        }
    }

    /// 放到队尾；保留 Result 接口以兼容现有消息生产者。
    pub fn enqueue(&self, item: T) -> Result<(), String> {
        self.items.lock().push_back(item);
        Ok(())
    }

    /// 取走队首消息；与撤回使用同一把短临界区锁，保证只会有一个胜者。
    pub fn try_next(&self) -> Option<T> {
        self.items.lock().pop_front()
    }

    /// 仅移除尚未被消费的消息，余下消息保持原有顺序。
    pub fn remove_first(&self, predicate: impl Fn(&T) -> bool) -> Option<T> {
        let mut items = self.items.lock();
        let pos = items.iter().position(predicate)?;
        items.remove(pos)
    }
}

impl<T: Send + 'static> Default for AsyncMessageQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn withdrawal_preserves_fifo_and_cannot_remove_consumed_messages() {
        let queue = AsyncMessageQueue::new();
        for id in 1..=3 {
            queue.enqueue(id).unwrap();
        }
        assert_eq!(queue.remove_first(|id| *id == 2), Some(2));
        assert_eq!(queue.try_next(), Some(1));
        assert_eq!(queue.remove_first(|id| *id == 1), None);
        assert_eq!(queue.try_next(), Some(3));
        assert_eq!(queue.try_next(), None);
    }

    #[test]
    fn consumption_and_withdrawal_have_exactly_one_winner() {
        let queue = std::sync::Arc::new(AsyncMessageQueue::new());
        queue.enqueue(7).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let consumer_queue = queue.clone();
        let consumer_barrier = barrier.clone();
        let consumer = std::thread::spawn(move || {
            consumer_barrier.wait();
            consumer_queue.try_next()
        });
        barrier.wait();
        let withdrawn = queue.remove_first(|id| *id == 7);
        let consumed = consumer.join().unwrap();
        assert_ne!(withdrawn.is_some(), consumed.is_some());
        assert_eq!(withdrawn.or(consumed), Some(7));
        assert_eq!(queue.try_next(), None);
    }
}
