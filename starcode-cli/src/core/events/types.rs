//! Event type definitions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Session events for event sourcing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    // Session lifecycle
    SessionCreated {
        session_id: String,
        timestamp: DateTime<Utc>,
    },
    SessionResumed {
        session_id: String,
        timestamp: DateTime<Utc>,
    },

    // Input events
    PromptAdmitted {
        session_id: String,
        message_id: String,
        prompt: Prompt,
        timestamp: DateTime<Utc>,
    },

    // Text streaming events
    TextStarted {
        session_id: String,
        message_id: String,
        text_id: String,
        timestamp: DateTime<Utc>,
    },
    TextDelta {
        session_id: String,
        message_id: String,
        text_id: String,
        delta: String,
        timestamp: DateTime<Utc>,
    },
    TextEnded {
        session_id: String,
        message_id: String,
        text_id: String,
        text: String,
        timestamp: DateTime<Utc>,
    },

    // Tool input streaming events
    ToolInputStarted {
        session_id: String,
        message_id: String,
        call_id: String,
        tool_name: String,
        timestamp: DateTime<Utc>,
    },
    ToolInputDelta {
        session_id: String,
        message_id: String,
        call_id: String,
        delta: String,
        timestamp: DateTime<Utc>,
    },
    ToolInputEnded {
        session_id: String,
        message_id: String,
        call_id: String,
        input: String,
        timestamp: DateTime<Utc>,
    },

    // Permission events
    PermissionRequested {
        session_id: String,
        request_id: String,
        action: String,
        resources: Vec<String>,
        timestamp: DateTime<Utc>,
    },
    PermissionReplied {
        session_id: String,
        request_id: String,
        reply: PermissionReply,
        timestamp: DateTime<Utc>,
    },

    // Question events
    QuestionAsked {
        session_id: String,
        question_id: String,
        questions: Vec<Question>,
        timestamp: DateTime<Utc>,
    },
    QuestionReplied {
        session_id: String,
        question_id: String,
        answers: Vec<Vec<String>>,
        timestamp: DateTime<Utc>,
    },

    // Tool execution events
    ToolCallStarted {
        session_id: String,
        message_id: String,
        call_id: String,
        tool_name: String,
        input: serde_json::Value,
        timestamp: DateTime<Utc>,
    },
    ToolCallCompleted {
        session_id: String,
        message_id: String,
        call_id: String,
        result: ToolResultStatus,
        timestamp: DateTime<Utc>,
    },
}

/// Prompt data for events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prompt {
    pub text: String,
    pub files: Vec<FileAttachment>,
    pub agents: Vec<AgentAttachment>,
}

/// File attachment in prompt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileAttachment {
    pub uri: String,
    pub mime: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Agent attachment in prompt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAttachment {
    pub name: String,
}

/// Permission reply options
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PermissionReply {
    /// Allow this time only
    Once,
    /// Allow and save rule for future
    Always,
    /// Reject the request
    Reject,
}

/// Question definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Question {
    pub text: String,
    pub header: String,
    pub options: Vec<QuestionOption>,
    pub multiple: bool,
    pub custom: bool,
}

/// Question option
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
}

/// Tool result status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToolResultStatus {
    Success { output: String },
    Error { message: String },
}

/// Permission request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    pub session_id: String,
    pub action: String,
    pub resources: Vec<String>,
    pub save: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

/// Question request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionRequest {
    pub id: String,
    pub session_id: String,
    pub questions: Vec<Question>,
}
