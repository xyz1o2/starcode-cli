/// 使用量适配器
/// 
/// 将不同Provider的使用量格式转换为统一格式

use super::types::UsageRecord;

/// 使用量适配器
pub struct UsageAdapter;

impl UsageAdapter {
    /// 从Anthropic响应创建使用量记录
    pub fn from_anthropic(
        provider: &str,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> UsageRecord {
        UsageRecord {
            id: uuid::Uuid::new_v4().to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            cost: Self::calculate_anthropic_cost(model, prompt_tokens, completion_tokens),
            timestamp: chrono::Utc::now().timestamp(),
            session_id: None,
            request_id: None,
        }
    }

    /// 从OpenAI响应创建使用量记录
    pub fn from_openai(
        provider: &str,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> UsageRecord {
        UsageRecord {
            id: uuid::Uuid::new_v4().to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            cost: Self::calculate_openai_cost(model, prompt_tokens, completion_tokens),
            timestamp: chrono::Utc::now().timestamp(),
            session_id: None,
            request_id: None,
        }
    }

    /// 从Gemini响应创建使用量记录
    pub fn from_gemini(
        provider: &str,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> UsageRecord {
        UsageRecord {
            id: uuid::Uuid::new_v4().to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            prompt_tokens,
            completion_tokens,
            total_tokens: prompt_tokens + completion_tokens,
            cost: Self::calculate_gemini_cost(model, prompt_tokens, completion_tokens),
            timestamp: chrono::Utc::now().timestamp(),
            session_id: None,
            request_id: None,
        }
    }

    /// 计算Anthropic成本
    fn calculate_anthropic_cost(model: &str, prompt_tokens: u32, completion_tokens: u32) -> Option<f64> {
        let (input_price, output_price) = match model {
            "claude-3-opus-20240229" => (15.0, 75.0),
            "claude-3-sonnet-20240229" => (3.0, 15.0),
            "claude-3-haiku-20240307" => (0.25, 1.25),
            "claude-3-5-sonnet-20241022" => (3.0, 15.0),
            _ => (3.0, 15.0), // 默认价格
        };

        let input_cost = (prompt_tokens as f64 / 1_000_000.0) * input_price;
        let output_cost = (completion_tokens as f64 / 1_000_000.0) * output_price;

        Some(input_cost + output_cost)
    }

    /// 计算OpenAI成本
    fn calculate_openai_cost(model: &str, prompt_tokens: u32, completion_tokens: u32) -> Option<f64> {
        let (input_price, output_price) = match model {
            "gpt-4o" => (5.0, 15.0),
            "gpt-4o-mini" => (0.15, 0.60),
            "gpt-4-turbo" => (10.0, 30.0),
            "gpt-4" => (30.0, 60.0),
            "gpt-3.5-turbo" => (0.50, 1.50),
            _ => (5.0, 15.0), // 默认价格
        };

        let input_cost = (prompt_tokens as f64 / 1_000_000.0) * input_price;
        let output_cost = (completion_tokens as f64 / 1_000_000.0) * output_price;

        Some(input_cost + output_cost)
    }

    /// 计算Gemini成本
    fn calculate_gemini_cost(model: &str, prompt_tokens: u32, completion_tokens: u32) -> Option<f64> {
        let (input_price, output_price) = match model {
            "gemini-1.5-pro" => (3.50, 10.50),
            "gemini-1.5-flash" => (0.075, 0.30),
            "gemini-1.0-pro" => (0.50, 1.50),
            _ => (3.50, 10.50), // 默认价格
        };

        let input_cost = (prompt_tokens as f64 / 1_000_000.0) * input_price;
        let output_cost = (completion_tokens as f64 / 1_000_000.0) * output_price;

        Some(input_cost + output_cost)
    }
}
