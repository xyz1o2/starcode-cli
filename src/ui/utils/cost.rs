/// Cost computation utilities for per-response cost display.
use crate::types::StarUsage;

/// Model-specific pricing ($ per 1M tokens)
struct ModelPricing {
    input_rate: f64,
    output_rate: f64,
}

/// Cache-specific pricing ($ per 1M tokens)
struct CachePricing {
    /// Cache read (hit) rate — typically much cheaper than input
    read_rate: f64,
    /// Cache write (creation) rate — typically more expensive than input
    write_rate: f64,
}

/// Get pricing for a model. Falls back to a reasonable default.
fn get_model_pricing(model: &str) -> ModelPricing {
    let lower = model.to_lowercase();

    // ── Claude models ──
    if lower.contains("claude-opus") || lower.contains("claude-4-opus") {
        return ModelPricing { input_rate: 15.0, output_rate: 75.0 };
    }
    if lower.contains("claude-sonnet") || lower.contains("claude-4-sonnet")
        || lower.contains("claude-3.5-sonnet") || lower.contains("claude-3-6-sonnet")
        || lower.contains("claude-3.7-sonnet") {
        return ModelPricing { input_rate: 3.0, output_rate: 15.0 };
    }
    if lower.contains("claude-haiku") || lower.contains("claude-3.5-haiku")
        || lower.contains("claude-3-haiku") || lower.contains("claude-4-haiku") {
        return ModelPricing { input_rate: 0.80, output_rate: 4.0 };
    }

    // ── GPT / OpenAI models ──
    if lower.contains("gpt-4.1-nano") {
        return ModelPricing { input_rate: 0.10, output_rate: 0.40 };
    }
    if lower.contains("gpt-4.1-mini") {
        return ModelPricing { input_rate: 0.40, output_rate: 1.60 };
    }
    if lower.contains("gpt-4.1") {
        return ModelPricing { input_rate: 2.0, output_rate: 8.0 };
    }
    if lower.contains("gpt-4o-mini") {
        return ModelPricing { input_rate: 0.15, output_rate: 0.60 };
    }
    if lower.contains("gpt-4o") {
        return ModelPricing { input_rate: 2.50, output_rate: 10.0 };
    }
    if lower.contains("gpt-4-turbo") {
        return ModelPricing { input_rate: 10.0, output_rate: 30.0 };
    }
    if lower.contains("o4-mini") {
        return ModelPricing { input_rate: 1.10, output_rate: 4.40 };
    }
    if lower.contains("o3-mini") {
        return ModelPricing { input_rate: 1.10, output_rate: 4.40 };
    }
    if lower.contains("o3") {
        return ModelPricing { input_rate: 10.0, output_rate: 40.0 };
    }
    if lower.contains("o1-mini") {
        return ModelPricing { input_rate: 3.0, output_rate: 12.0 };
    }
    if lower.contains("o1") {
        return ModelPricing { input_rate: 15.0, output_rate: 60.0 };
    }

    // ── DeepSeek ──
    if lower.contains("deepseek") {
        return ModelPricing { input_rate: 0.27, output_rate: 1.10 };
    }

    // ── Gemini ──
    if lower.contains("gemini-2.5-pro") {
        return ModelPricing { input_rate: 1.25, output_rate: 10.0 };
    }
    if lower.contains("gemini-2.5-flash") {
        return ModelPricing { input_rate: 0.15, output_rate: 0.60 };
    }
    if lower.contains("gemini-2.0-flash") {
        return ModelPricing { input_rate: 0.10, output_rate: 0.40 };
    }
    if lower.contains("gemini-pro") || lower.contains("gemini-1.5-pro") {
        return ModelPricing { input_rate: 1.25, output_rate: 5.0 };
    }
    if lower.contains("gemini-flash") || lower.contains("gemini-1.5-flash") {
        return ModelPricing { input_rate: 0.075, output_rate: 0.30 };
    }

    // ── Qwen ──
    if lower.contains("qwen-max") {
        return ModelPricing { input_rate: 2.40, output_rate: 9.60 };
    }
    if lower.contains("qwen-plus") {
        return ModelPricing { input_rate: 0.30, output_rate: 1.20 };
    }
    if lower.contains("qwen-turbo") {
        return ModelPricing { input_rate: 0.05, output_rate: 0.20 };
    }
    if lower.contains("qwen") {
        return ModelPricing { input_rate: 0.30, output_rate: 1.20 };
    }

    // ── Llama / Meta ──
    if lower.contains("llama-4") {
        return ModelPricing { input_rate: 0.25, output_rate: 1.0 };
    }
    if lower.contains("llama-3.3") || lower.contains("llama-3.1") {
        return ModelPricing { input_rate: 0.20, output_rate: 0.60 };
    }
    if lower.contains("llama") {
        return ModelPricing { input_rate: 0.20, output_rate: 0.60 };
    }

    // ── Mistral ──
    if lower.contains("mistral-large") {
        return ModelPricing { input_rate: 2.0, output_rate: 6.0 };
    }
    if lower.contains("mistral-small") || lower.contains("mistral-medium") {
        return ModelPricing { input_rate: 0.20, output_rate: 0.60 };
    }
    if lower.contains("mistral") {
        return ModelPricing { input_rate: 0.25, output_rate: 1.0 };
    }

    // ── Grok ──
    if lower.contains("grok-3") {
        return ModelPricing { input_rate: 3.0, output_rate: 15.0 };
    }
    if lower.contains("grok") {
        return ModelPricing { input_rate: 0.50, output_rate: 1.50 };
    }

    // ── Default fallback (conservative) ──
    ModelPricing { input_rate: 1.0, output_rate: 3.0 }
}

/// Get cache-specific pricing for a model.
/// Different providers have very different cache pricing.
fn get_cache_pricing(model: &str, base_input_rate: f64) -> CachePricing {
    let lower = model.to_lowercase();

    // Anthropic: cache read = 90% off input, cache write = 25% more than input
    if lower.contains("claude") {
        return CachePricing {
            read_rate: base_input_rate * 0.1,
            write_rate: base_input_rate * 1.25,
        };
    }

    // DeepSeek: cache read = ~1% of input (extremely cheap), cache write = same as input
    if lower.contains("deepseek") {
        return CachePricing {
            read_rate: 0.07,  // $0.07/1M fixed
            write_rate: base_input_rate,
        };
    }

    // OpenAI (GPT-4o, etc.): cache read = 50% off input, cache write = same as input
    if lower.contains("gpt-4o") || lower.contains("gpt-4.1") {
        return CachePricing {
            read_rate: base_input_rate * 0.5,
            write_rate: base_input_rate,
        };
    }

    // Gemini: cache read = 75% off, cache write = same as input
    if lower.contains("gemini") {
        return CachePricing {
            read_rate: base_input_rate * 0.25,
            write_rate: base_input_rate,
        };
    }

    // Default: no cache discount
    CachePricing {
        read_rate: base_input_rate,
        write_rate: base_input_rate,
    }
}

/// Compute the cost of a single response given token usage and model name.
pub fn compute_response_cost(usage: &StarUsage, model: &str) -> f64 {
    let pricing = get_model_pricing(model);
    let cache_pricing = get_cache_pricing(model, pricing.input_rate);

    // Separate regular input tokens from cache tokens
    // prompt_tokens may include cache tokens, so we subtract them
    let cache_total = usage.cache_read_tokens + usage.cache_creation_tokens;
    let regular_input = if usage.prompt_tokens > cache_total {
        usage.prompt_tokens - cache_total
    } else {
        // If cache tokens exceed prompt_tokens (edge case), treat all as regular
        usage.prompt_tokens
    };

    let input_cost = (regular_input as f64) * pricing.input_rate / 1_000_000.0
        + (usage.cache_read_tokens as f64) * cache_pricing.read_rate / 1_000_000.0
        + (usage.cache_creation_tokens as f64) * cache_pricing.write_rate / 1_000_000.0;
    let output_cost = (usage.completion_tokens as f64) * pricing.output_rate / 1_000_000.0;

    input_cost + output_cost
}
