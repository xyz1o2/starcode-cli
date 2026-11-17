//! OpenAI 兼容网关的流式帧修正。
//!
//! 部分网关把增量或统计帧标为 `chat.completion`，rig 会误走完整响应解码。
//! 仅在流请求中修正可判定的增量形状；完整回答和错误封套保持原样。

use futures::StreamExt;
use rig_core::driver::{Exchange, Opening, Transport};
use rig_core::wire::{Encoded, Wire, WireFrame};
use serde_json::Value;

#[derive(Clone)]
pub(super) struct ChatStreamTransport<H>(pub H);

impl<W, H> Transport<W> for ChatStreamTransport<H>
where
    W: Wire<Payload = Encoded, Frame = WireFrame>,
    H: Transport<W>,
{
    fn send(&self, payload: Encoded, exchange: Exchange) -> Opening<WireFrame> {
        let streaming = exchange.mode == rig_core::wire::Mode::Streaming;
        let opening = self.0.send(payload, exchange);
        Opening::new(async move {
            let opened = opening.await?;
            if !streaming {
                return Ok(opened);
            }
            Ok(opened.map_frames(|frames| {
                let mut saw_choice = false;
                frames.map(move |frame| frame.map(|frame| normalize_frame(frame, &mut saw_choice)))
            }))
        })
    }
}

fn normalize_frame(frame: WireFrame, saw_choice: &mut bool) -> WireFrame {
    let Ok(mut value) = serde_json::from_str::<Value>(&frame.as_str()) else {
        return frame;
    };
    // 统计帧只能补充已出现的回答，不能把一条没有回答的流变成成功。
    if value
        .get("choices")
        .and_then(Value::as_array)
        .is_some_and(|choices| {
            choices.iter().any(|choice| {
                choice.get("delta").is_some_and(Value::is_object)
                    || choice.get("message").is_some_and(Value::is_object)
            })
        })
    {
        *saw_choice = true;
    }
    if value.get("object").and_then(Value::as_str) != Some("chat.completion")
        || value.get("error").is_some_and(|error| !error.is_null())
    {
        return frame;
    }
    let Some(choices) = value.get("choices").and_then(Value::as_array) else {
        crate::utils::logging::append_debug_log_line(
            "[LLM] chat.completion stream frame has no choices array; preserving invalid response",
        );
        return frame;
    };
    let deltas = !choices.is_empty()
        && choices.iter().all(|choice| {
            choice.get("message").is_none() && choice.get("delta").is_some_and(Value::is_object)
        });
    let usage_only =
        *saw_choice && choices.is_empty() && value.get("usage").is_some_and(Value::is_object);
    if !deltas && !usage_only {
        if choices.is_empty() {
            crate::utils::logging::append_debug_log_line(
                "[LLM] chat.completion stream frame has no usable choice; preserving invalid response",
            );
        }
        return frame;
    }
    value["object"] = Value::String("chat.completion.chunk".into());
    crate::utils::logging::append_debug_log_line(
        "[LLM] normalized mislabeled chat stream frame (completion -> chunk)",
    );
    WireFrame::Bytes(serde_json::to_vec(&value).expect("JSON value serializes"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmClient, LlmEvent};

    #[test]
    fn complete_empty_and_error_responses_are_unchanged() {
        for raw in [
            r#"{"object":"chat.completion","choices":[]}"#,
            r#"{"object":"chat.completion","choices":[{"message":{"role":"assistant","content":"ok"}}]}"#,
            r#"{"object":"chat.completion","choices":[],"usage":{},"error":{"message":"denied"}}"#,
            r#"{"object":"chat.completion.chunk","choices":[],"usage":{}}"#,
        ] {
            assert_eq!(
                normalize_frame(WireFrame::Bytes(raw.into()), &mut false).as_str(),
                raw
            );
        }
    }

    async fn mock_stream(frames: Vec<Value>) -> Vec<Result<LlmEvent, crate::llm::LlmError>> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]);
                    let len = headers
                        .lines()
                        .find_map(|line| {
                            let (key, value) = line.split_once(':')?;
                            key.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    if request.len() >= end + 4 + len {
                        let body: Value =
                            serde_json::from_slice(&request[end + 4..end + 4 + len]).unwrap();
                        assert_eq!(body["stream"], true);
                        break;
                    }
                }
            }
            let mut body = frames
                .into_iter()
                .map(|frame| format!("data: {frame}\n\n"))
                .collect::<String>();
            body.push_str("data: [DONE]\n\n");
            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let adapter = crate::llm::rig_adapter::RigAdapter::openai_compatible(
            "test-key".into(),
            "test-model".into(),
            format!("http://{addr}/v1"),
            "local-test".into(),
        );
        let stream = adapter
            .chat_stream_events(vec![crate::types::StarMessage::user("hello")], None)
            .await
            .unwrap();
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), stream.collect())
            .await
            .unwrap();
        server.await.unwrap();
        result
    }

    #[tokio::test]
    async fn dropping_stream_releases_pending_http_connection() {
        assert_dropped_http_connection().await;
    }

    async fn assert_dropped_http_connection() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for send_headers in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let (started_tx, started_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buf = [0; 4096];
                loop {
                    let count = socket.read(&mut buf).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buf[..count]);
                    if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        let length = String::from_utf8_lossy(&request[..end])
                            .lines()
                            .find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length")
                                    .then(|| value.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                if send_headers {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"object\":\"chat.completion.chunk\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"working\"}}]}\n\n").await.unwrap();
                }
                let _ = started_tx.send(());
                let closed =
                    tokio::time::timeout(std::time::Duration::from_secs(3), socket.read(&mut buf))
                        .await
                        .unwrap_or_else(|_| {
                            panic!(
                                "dropping the consumer must release HTTP: headers={send_headers}"
                            )
                        });
                assert!(matches!(closed, Ok(0) | Err(_)));
            });
            struct StopServer(tokio::task::AbortHandle);
            impl Drop for StopServer {
                fn drop(&mut self) {
                    self.0.abort();
                }
            }
            let _guard = StopServer(server.abort_handle());
            let adapter = crate::llm::rig_adapter::RigAdapter::openai_compatible(
                "test-key".into(),
                "test-model".into(),
                format!("http://{addr}/v1"),
                "local-test".into(),
            );
            let mut stream = adapter
                .chat_stream_events(vec![crate::types::StarMessage::user("hello")], None)
                .await
                .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(5), started_rx)
                .await
                .unwrap()
                .unwrap();
            if send_headers {
                let first = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert!(matches!(first, LlmEvent::TextChunk(_)));
            }
            drop(stream);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn mislabeled_usage_frame_keeps_text_and_usage_without_duplicate_text() {
        let events = mock_stream(vec![
            serde_json::json!({"object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"hello "}}]}),
            serde_json::json!({"object":"chat.completion","choices":[{"index":0,"delta":{"content":"hello "}}]}),
            serde_json::json!({"object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            serde_json::json!({"object":"chat.completion","choices":[],"usage":{"prompt_tokens":3,"completion_tokens":2,"total_tokens":5}}),
        ]).await;
        let mut text = String::new();
        let mut tokens = None;
        let mut finished = false;
        for event in events {
            match event.expect("valid SSE deltas and usage must decode") {
                LlmEvent::TextChunk(chunk) => text.push_str(&chunk),
                LlmEvent::UsageUpdate(usage) => tokens = Some(usage.total_tokens),
                LlmEvent::Finish(_) => finished = true,
                _ => {}
            }
        }
        assert_eq!(text, "hello hello ");
        assert_eq!(tokens, Some(5));
        assert!(finished);
    }

    #[tokio::test]
    async fn actual_empty_completion_is_still_an_error() {
        for reply in [
            serde_json::json!({"object":"chat.completion","choices":[]}),
            serde_json::json!({"object":"chat.completion","choices":[],"usage":{"total_tokens":5}}),
        ] {
            let events = mock_stream(vec![reply]).await;
            assert!(events.iter().any(|event| event
                .as_ref()
                .err()
                .is_some_and(|error| error.to_string().contains("no choices"))));
            assert!(!events
                .iter()
                .any(|event| matches!(event, Ok(LlmEvent::Finish(_)))));
        }
    }
}
