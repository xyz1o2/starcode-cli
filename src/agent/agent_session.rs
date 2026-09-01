use crate::agent::agent_core::Agent;
use crate::agent::session;
use crate::types::StarMessage;

impl Agent {
    pub fn replace_session_messages(&mut self, messages: Vec<StarMessage>) {
        self.sync_and_persist(messages);
    }

    pub fn push_message(&mut self, message: StarMessage) {
        self.session_messages.push(message);
    }

    pub fn clear_session_messages(&mut self) {
        self.session_messages.clear();
        self.persist_session_messages_to_disk();
    }

    pub async fn force_compress_session_messages(
        &mut self,
    ) -> Result<
        crate::agent::workflows::context_compression::CompressionResult,
        Box<dyn std::error::Error + Send + Sync>,
    > {
        if self.session_messages.is_empty() {
            return Ok(
                crate::agent::workflows::context_compression::CompressionResult {
                    messages: Vec::new(),
                    was_compacted: false,
                    original_token_count: 0,
                    new_token_count: 0,
                    threshold_tokens: 0,
                    decision: "empty_session",
                },
            );
        }

        let result = self
            .context_compressor
            .force_compress(self.session_messages.clone(), Some(&self.client))
            .await?;
        self.session_messages = result.messages.clone();
        self.persist_session_messages();
        Ok(result)
    }

    pub fn append_external_tool_result(
        &mut self,
        tool_call: &crate::types::StarToolCall,
        tool_result: &crate::types::ToolResult,
    ) {
        if self.session_messages.is_empty() {
            self.session_messages.push(StarMessage::system(
                "Session context initialized.".to_string(),
            ));
        }

        self.session_messages
            .push(StarMessage::assistant_with_tool_calls(vec![
                tool_call.clone()
            ]));
        self.session_messages.push(StarMessage::tool(
            tool_call.id.clone(),
            tool_result
                .output
                .clone()
                .or_else(|| tool_result.error.clone())
                .unwrap_or_default(),
        ));

        self.persist_session_messages();
    }

    pub(crate) fn persist_session_messages(&mut self) {
        // 子代理不持久化会话消息（避免覆盖父代理的 session_messages.json）
        if self.config.recursion_depth > 0 {
            return;
        }
        session::persist_session_messages(
            &self.session_messages,
            self.config.storage().session_messages_path(),
        );
    }

    pub(crate) fn sync_and_persist(&mut self, messages: Vec<StarMessage>) {
        self.session_messages = messages;
        self.persist_session_messages();
    }

    pub(crate) fn persist_session_messages_to_disk(&self) {
        session::persist_session_messages_to_disk(
            &self.session_messages,
            self.config.storage().session_messages_path(),
        );
    }

    pub(crate) fn load_persisted_session_messages(&mut self) {
        self.session_messages = session::load_persisted_session_messages(
            &self.config.storage().session_messages_path(),
        );
    }
}
