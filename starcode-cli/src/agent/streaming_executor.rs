use crate::agent::tool_executor::ToolExecutor;
use crate::types::{StarToolCall, ToolResult};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio_util::sync::CancellationToken;

/// 工具取消原因
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolAbortReason {
    /// 兄弟工具错误（Bash错误级联）
    SiblingError,
    /// 用户中断
    UserInterrupted,
    /// 流式回退
    StreamingFallback,
}

/// Progress消息
#[derive(Debug, Clone)]
pub struct ToolProgressMessage {
    /// 工具调用ID
    pub tool_use_id: String,
    /// 进度数据
    pub data: ProgressData,
}

/// 进度数据类型
#[derive(Debug, Clone)]
pub enum ProgressData {
    /// Bash命令进度
    BashProgress {
        /// 输出行
        output: String,
        /// 是否是stderr
        is_stderr: bool,
    },
    /// 文件操作进度
    FileProgress {
        /// 当前文件
        current_file: String,
        /// 进度百分比
        percent: Option<u32>,
    },
    /// 通用进度消息
    Message {
        /// 消息内容
        content: String,
    },
}

/// 工具执行span（用于Langfuse可观测性）
#[derive(Debug, Clone)]
pub struct ToolExecutionSpan {
    /// span ID
    pub id: String,
    /// 工具名称
    pub tool_name: String,
    /// 开始时间
    pub started_at: std::time::Instant,
    /// 父span ID
    pub parent_span_id: Option<String>,
}

/// 流式工具执行器
/// 
/// 对标claude-code-main的StreamingToolExecutor
/// 在模型还在生成响应时就开始执行工具调用
/// 减少整体延迟，特别是对于只读工具
pub struct StreamingToolExecutor {
    tool_executor: Arc<ToolExecutor>,
    max_concurrent: usize,
    /// 并发信号量
    semaphore: Arc<Semaphore>,
    /// 待执行的工具调用队列
    pending_calls: Vec<PendingToolCall>,
    /// 正在执行的工具调用
    in_progress: HashMap<String, InProgressToolCall>,
    /// 已完成的工具结果
    completed_results: HashMap<String, ToolResult>,
    /// 是否有错误发生（用于Bash错误级联）
    has_errored: Arc<AtomicBool>,
    /// 错误工具描述
    errored_tool_description: String,
    /// 兄弟工具取消信号
    sibling_abort_token: CancellationToken,
    /// Progress消息发送通道
    progress_tx: mpsc::UnboundedSender<ToolProgressMessage>,
    /// Progress消息接收通道
    progress_rx: mpsc::UnboundedReceiver<ToolProgressMessage>,
    /// 执行span存储
    spans: HashMap<String, ToolExecutionSpan>,
    /// 是否已丢弃
    discarded: bool,
}

struct PendingToolCall {
    tool_call: StarToolCall,
    result_tx: oneshot::Sender<ToolResult>,
}

struct InProgressToolCall {
    tool_call: StarToolCall,
    result_rx: oneshot::Receiver<ToolResult>,
}

impl StreamingToolExecutor {
    pub fn new(tool_executor: Arc<ToolExecutor>, max_concurrent: usize) -> Self {
        let (progress_tx, progress_rx) = mpsc::unbounded_channel();
        Self {
            tool_executor,
            max_concurrent: max_concurrent.max(1),
            semaphore: Arc::new(Semaphore::new(max_concurrent.max(1))),
            pending_calls: Vec::new(),
            in_progress: HashMap::new(),
            completed_results: HashMap::new(),
            has_errored: Arc::new(AtomicBool::new(false)),
            errored_tool_description: String::new(),
            sibling_abort_token: CancellationToken::new(),
            progress_tx,
            progress_rx,
            spans: HashMap::new(),
            discarded: false,
        }
    }

    /// 尝试获取Progress消息（非阻塞）
    pub fn try_recv_progress(&mut self) -> Option<ToolProgressMessage> {
        self.progress_rx.try_recv().ok()
    }

    /// 获取所有可用的Progress消息
    pub fn drain_progress(&mut self) -> Vec<ToolProgressMessage> {
        let mut messages = Vec::new();
        while let Ok(msg) = self.progress_rx.try_recv() {
            messages.push(msg);
        }
        messages
    }

    /// 发送Progress消息
    pub fn send_progress(&self, message: ToolProgressMessage) {
        let _ = self.progress_tx.send(message);
    }

    /// 检查是否是Bash工具
    fn is_bash_tool(tool_name: &str) -> bool {
        tool_name == "Bash" || tool_name == "bash" || tool_name == "shell"
    }

    /// 获取取消原因
    fn get_abort_reason(&self, tool_id: &str) -> Option<ToolAbortReason> {
        // 检查是否是流式回退
        if self.sibling_abort_token.is_cancelled() && !self.has_errored.load(Ordering::Relaxed) {
            return Some(ToolAbortReason::StreamingFallback);
        }

        // 检查是否有兄弟工具错误
        if self.has_errored.load(Ordering::Relaxed) {
            // 不取消导致错误的工具本身
            if !self.errored_tool_description.contains(tool_id) {
                return Some(ToolAbortReason::SiblingError);
            }
        }

        None
    }

    /// 创建合成错误消息
    fn create_synthetic_error_result(&self, tool_id: &str, reason: &ToolAbortReason) -> ToolResult {
        let error_msg = match reason {
            ToolAbortReason::SiblingError => {
                if self.errored_tool_description.is_empty() {
                    "Cancelled: parallel tool call errored".to_string()
                } else {
                    format!("Cancelled: parallel tool call {} errored", self.errored_tool_description)
                }
            }
            ToolAbortReason::UserInterrupted => "User interrupted tool execution".to_string(),
            ToolAbortReason::StreamingFallback => "Streaming fallback - tool execution discarded".to_string(),
        };

        ToolResult {
            success: false,
            output: None,
            error: Some(error_msg),
            data: None,
        }
    }

    /// 检查工具调用是否完整（参数已完整）
    pub fn is_call_complete(tool_call: &StarToolCall) -> bool {
        // 检查参数是否是有效的JSON对象
        if tool_call.function.arguments.is_empty() || tool_call.function.arguments == "{}" {
            return false;
        }
        
        // 尝试解析参数
        match serde_json::from_str::<serde_json::Value>(&tool_call.function.arguments) {
            Ok(value) => {
                // 检查是否是完整的对象（不是null或空）
                value.is_object() && !value.as_object().map_or(true, |o| o.is_empty())
            }
            Err(_) => false,
        }
    }

    /// 提交工具调用进行流式执行
    /// 
    /// 如果工具调用参数已完整，立即开始执行
    /// 否则等待参数完整后再执行
    pub fn submit_call(&mut self, tool_call: StarToolCall) -> Option<tokio::task::JoinHandle<()>> {
        if self.discarded {
            return None;
        }

        // 检查是否已经有这个工具调用的结果
        if self.completed_results.contains_key(&tool_call.id) {
            return None;
        }

        // 检查是否已经在执行中
        if self.in_progress.contains_key(&tool_call.id) {
            return None;
        }

        // 检查工具调用是否完整
        if !Self::is_call_complete(&tool_call) {
            // 参数不完整，加入待执行队列
            let (result_tx, _result_rx) = oneshot::channel();
            self.pending_calls.push(PendingToolCall {
                tool_call,
                result_tx,
            });
            return None;
        }

        // 参数完整，立即开始执行
        self.execute_call(tool_call)
    }

    /// 执行工具调用
    fn execute_call(&mut self, tool_call: StarToolCall) -> Option<tokio::task::JoinHandle<()>> {
        let tool_id = tool_call.id.clone();
        let tool_name = tool_call.function.name.clone();
        let tool_executor = self.tool_executor.clone();
        let call_clone = tool_call.clone();
        let has_errored = self.has_errored.clone();
        let sibling_abort_token = self.sibling_abort_token.clone();
        let semaphore = self.semaphore.clone();
        let progress_tx = self.progress_tx.clone();

        // 创建结果通道
        let (result_tx, result_rx) = oneshot::channel();

        // 记录正在执行的工具调用
        self.in_progress.insert(tool_id.clone(), InProgressToolCall {
            tool_call: tool_call.clone(),
            result_rx,
        });

        // 创建执行span
        let span = ToolExecutionSpan {
            id: uuid::Uuid::new_v4().to_string(),
            tool_name: tool_name.clone(),
            started_at: std::time::Instant::now(),
            parent_span_id: None,
        };
        self.spans.insert(tool_id.clone(), span);

        // 启动异步执行任务
        let handle = tokio::spawn(async move {
            // 获取并发许可
            let _permit = semaphore.acquire().await.unwrap();

            // 检查是否应该取消
            if sibling_abort_token.is_cancelled() {
                let _ = result_tx.send(ToolResult {
                    success: false,
                    output: None,
                    error: Some("Cancelled by sibling error".to_string()),
                    data: None,
                });
                return;
            }

            // 发送开始进度
            let _ = progress_tx.send(ToolProgressMessage {
                tool_use_id: tool_id.clone(),
                data: ProgressData::Message {
                    content: format!("Executing {}...", tool_name),
                },
            });

            let result = tool_executor.execute(&call_clone, None, None).await;
            
            // 检查是否是Bash工具错误
            if !result.success && Self::is_bash_tool(&tool_name) {
                has_errored.store(true, Ordering::Relaxed);
                sibling_abort_token.cancel();
            }

            // 发送完成进度
            let _ = progress_tx.send(ToolProgressMessage {
                tool_use_id: tool_id.clone(),
                data: ProgressData::Message {
                    content: if result.success {
                        format!("{} completed successfully", tool_name)
                    } else {
                        format!("{} failed", tool_name)
                    },
                },
            });

            let _ = result_tx.send(result);
        });

        Some(handle)
    }

    /// 尝试获取已完成的工具结果
    pub fn try_get_result(&mut self, tool_id: &str) -> Option<ToolResult> {
        // 先检查已完成的结果
        if let Some(result) = self.completed_results.remove(tool_id) {
            // 清理span
            self.spans.remove(tool_id);
            return Some(result);
        }

        // 检查正在执行的工具调用
        if let Some(mut in_progress) = self.in_progress.remove(tool_id) {
            // 尝试非阻塞地获取结果
            match in_progress.result_rx.try_recv() {
                Ok(result) => {
                    self.completed_results.insert(tool_id.to_string(), result.clone());
                    // 清理span
                    self.spans.remove(tool_id);
                    Some(result)
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    // 还没完成，放回去
                    self.in_progress.insert(tool_id.to_string(), in_progress);
                    None
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    // 通道关闭，返回错误结果
                    // 清理span
                    self.spans.remove(tool_id);
                    Some(ToolResult {
                        success: false,
                        output: None,
                        error: Some("Tool execution channel closed".to_string()),
                        data: None,
                    })
                }
            }
        } else {
            None
        }
    }

    /// 等待工具执行完成
    pub async fn wait_for_result(&mut self, tool_id: &str) -> Option<ToolResult> {
        // 先检查已完成的结果
        if let Some(result) = self.completed_results.remove(tool_id) {
            // 清理span
            self.spans.remove(tool_id);
            return Some(result);
        }

        // 等待正在执行的工具调用
        if let Some(mut in_progress) = self.in_progress.remove(tool_id) {
            match in_progress.result_rx.await {
                Ok(result) => {
                    self.completed_results.insert(tool_id.to_string(), result.clone());
                    // 清理span
                    self.spans.remove(tool_id);
                    Some(result)
                }
                Err(_) => {
                    // 清理span
                    self.spans.remove(tool_id);
                    Some(ToolResult {
                        success: false,
                        output: None,
                        error: Some("Tool execution channel closed".to_string()),
                        data: None,
                    })
                }
            }
        } else {
            None
        }
    }

    /// 执行所有待执行的工具调用
    pub fn execute_pending(&mut self) -> Vec<tokio::task::JoinHandle<()>> {
        let mut handles = Vec::new();
        
        // 收集可以执行的待执行工具调用
        let pending: Vec<PendingToolCall> = self.pending_calls.drain(..).collect();
        
        for pending_call in pending {
            if Self::is_call_complete(&pending_call.tool_call) {
                if let Some(handle) = self.execute_call(pending_call.tool_call) {
                    handles.push(handle);
                }
            } else {
                // 参数仍然不完整，放回队列
                self.pending_calls.push(pending_call);
            }
        }
        
        handles
    }

    /// 获取所有已完成的结果
    pub fn drain_completed(&mut self) -> Vec<(String, ToolResult)> {
        let results: Vec<_> = self.completed_results.drain().collect();
        // 清理所有span
        self.spans.clear();
        results
    }

    /// 丢弃所有待执行和正在执行的工具调用
    pub fn discard(&mut self) {
        self.discarded = true;
        self.pending_calls.clear();
        self.in_progress.clear();
        self.completed_results.clear();
        self.spans.clear();
        self.sibling_abort_token.cancel();
    }

    /// 获取正在执行的工具调用数量
    pub fn in_progress_count(&self) -> usize {
        self.in_progress.len()
    }

    /// 获取待执行的工具调用数量
    pub fn pending_count(&self) -> usize {
        self.pending_calls.len()
    }

    /// 获取已完成的工具结果数量
    pub fn completed_count(&self) -> usize {
        self.completed_results.len()
    }

    /// 获取所有执行span
    pub fn get_spans(&self) -> &HashMap<String, ToolExecutionSpan> {
        &self.spans
    }

    /// 分区执行工具调用
    /// 
    /// 对标claude-code-main的toolOrchestration.ts
    /// 将工具调用分为只读和写入两组，只读工具并发执行，写入工具串行执行
    pub async fn execute_partitioned(
        &mut self,
        tool_calls: Vec<StarToolCall>,
        abort_signal: Option<CancellationToken>,
    ) -> Vec<ToolResult> {
        use futures::future::join_all;
        
        let mut results = Vec::with_capacity(tool_calls.len());
        let mut readonly_calls = Vec::new();
        let mut write_calls = Vec::new();
        let mut readonly_indices = Vec::new();
        let mut write_indices = Vec::new();
        
        // 分区工具调用
        for (i, call) in tool_calls.into_iter().enumerate() {
            if self.tool_executor.is_tool_read_only(&call.function.name) {
                readonly_calls.push(call);
                readonly_indices.push(i);
            } else {
                write_calls.push(call);
                write_indices.push(i);
            }
        }
        
        // 初始化结果数组
        results.resize_with(readonly_calls.len() + write_calls.len(), || {
            ToolResult {
                success: false,
                output: None,
                error: Some("Not executed".to_string()),
                data: None,
            }
        });
        
        // 并发执行只读工具
        if !readonly_calls.is_empty() {
            let futures: Vec<_> = readonly_calls
                .iter()
                .map(|call| {
                    let executor = self.tool_executor.clone();
                    let call_clone = call.clone();
                    let abort = abort_signal.clone();
                    async move {
                        executor.execute(&call_clone, None, abort).await
                    }
                })
                .collect();
            
            let readonly_results = join_all(futures).await;
            for (i, result) in readonly_results.into_iter().enumerate() {
                results[readonly_indices[i]] = result;
            }
        }
        
        // 串行执行写入工具
        for (i, call) in write_calls.iter().enumerate() {
            let result = self.tool_executor.execute(call, None, abort_signal.clone()).await;
            results[write_indices[i]] = result;
        }
        
        results
    }
}
