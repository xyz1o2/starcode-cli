use super::{LlmClient, LlmError, LlmEvent};
use crate::types::{StarChoice, StarMessage, StarResponse, StarTool};
use async_stream::stream;
use async_trait::async_trait;
use futures::Stream;
use std::error::Error;
use std::pin::Pin;

#[derive(Debug)]
pub struct MockClient {
    pub response_text: String,
}

impl MockClient {
    pub fn new(response_text: Option<String>) -> Self {
        Self {
            response_text: response_text.unwrap_or_else(|| "This is a mock response.".to_string()),
        }
    }
}

#[async_trait]
impl LlmClient for MockClient {
    async fn chat_completion(
        &self,
        _messages: Vec<StarMessage>,
        _tools: Option<Vec<StarTool>>,
    ) -> Result<StarResponse, Box<dyn Error + Send + Sync>> {
        Ok(StarResponse {
            choices: vec![StarChoice {
                message: StarMessage::assistant(self.response_text.clone()),
                finish_reason: "stop".to_string(),
            }],
            usage: None,
        })
    }

    async fn chat_stream_events(
        &self,
        _messages: Vec<StarMessage>,
        _tools: Option<Vec<StarTool>>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send + Sync>>, LlmError>
    {
        let response_text = self.response_text.clone();

        Ok(Box::pin(stream! {
            // Simulate streaming word by word
            for word in response_text.split_inclusive(' ') {
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                yield Ok(LlmEvent::TextChunk(word.to_string()));
            }

            yield Ok(LlmEvent::Finish("stop".to_string()));
        }))
    }

    fn get_model_info(&self) -> Option<crate::llm::ModelInfo> {
        None
    }
}
