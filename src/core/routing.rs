use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub model: String,
    pub metadata: RoutingMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingMetadata {
    pub source: String,
    pub reasoning: String,
    pub latency_ms: u64,
}

#[derive(Debug, Clone)]
pub struct RoutingContext {
    pub history_length: usize,
    pub request_complexity: RequestComplexity,
    pub user_override: Option<String>,
    pub default_model: String,
    pub fast_model: Option<String>,
    pub cheap_model: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestComplexity {
    Simple,
    Medium,
    Complex,
}

pub trait RoutingStrategy: Send + Sync {
    fn name(&self) -> &str;
    fn route(&self, context: &RoutingContext) -> Option<RoutingDecision>;
}

#[derive(Clone)]
pub struct UserOverrideStrategy;

impl RoutingStrategy for UserOverrideStrategy {
    fn name(&self) -> &str {
        "user_override"
    }

    fn route(&self, context: &RoutingContext) -> Option<RoutingDecision> {
        context.user_override.as_ref().map(|model| RoutingDecision {
            model: model.clone(),
            metadata: RoutingMetadata {
                source: "user_override".to_string(),
                reasoning: "User specified model".to_string(),
                latency_ms: 0,
            },
        })
    }
}

#[derive(Clone)]
pub struct PerformanceStrategy;

impl RoutingStrategy for PerformanceStrategy {
    fn name(&self) -> &str {
        "performance"
    }

    fn route(&self, context: &RoutingContext) -> Option<RoutingDecision> {
        let default_model = context.default_model.clone();
        let fast_model = context
            .fast_model
            .clone()
            .unwrap_or_else(|| default_model.clone());
        match context.request_complexity {
            RequestComplexity::Simple => Some(RoutingDecision {
                model: fast_model,
                metadata: RoutingMetadata {
                    source: "performance".to_string(),
                    reasoning: "Simple request, using fast model".to_string(),
                    latency_ms: 0,
                },
            }),
            RequestComplexity::Medium => Some(RoutingDecision {
                model: default_model.clone(),
                metadata: RoutingMetadata {
                    source: "performance".to_string(),
                    reasoning: "Medium complexity, using balanced model".to_string(),
                    latency_ms: 0,
                },
            }),
            RequestComplexity::Complex => Some(RoutingDecision {
                model: default_model,
                metadata: RoutingMetadata {
                    source: "performance".to_string(),
                    reasoning: "Complex request, using default/high-quality model".to_string(),
                    latency_ms: 0,
                },
            }),
        }
    }
}

#[derive(Clone)]
pub struct CostOptimizationStrategy;

impl RoutingStrategy for CostOptimizationStrategy {
    fn name(&self) -> &str {
        "cost_optimization"
    }

    fn route(&self, context: &RoutingContext) -> Option<RoutingDecision> {
        let cheap_model = context
            .cheap_model
            .clone()
            .unwrap_or_else(|| context.default_model.clone());
        if context.history_length < 5 {
            Some(RoutingDecision {
                model: cheap_model,
                metadata: RoutingMetadata {
                    source: "cost_optimization".to_string(),
                    reasoning: "Short conversation, using cost-optimized model".to_string(),
                    latency_ms: 0,
                },
            })
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub struct DefaultStrategy;

impl RoutingStrategy for DefaultStrategy {
    fn name(&self) -> &str {
        "default"
    }

    fn route(&self, _context: &RoutingContext) -> Option<RoutingDecision> {
        Some(RoutingDecision {
            model: _context.default_model.clone(),
            metadata: RoutingMetadata {
                source: "default".to_string(),
                reasoning: "Using default model".to_string(),
                latency_ms: 0,
            },
        })
    }
}

#[derive(Clone)]
pub struct RoutingEngine {
    strategies: Vec<Arc<dyn RoutingStrategy>>,
}

impl RoutingEngine {
    pub fn new() -> Self {
        Self {
            strategies: vec![
                Arc::new(UserOverrideStrategy),
                Arc::new(PerformanceStrategy),
                Arc::new(CostOptimizationStrategy),
                Arc::new(DefaultStrategy),
            ],
        }
    }

    pub fn route(&self, context: &RoutingContext) -> RoutingDecision {
        for strategy in &self.strategies {
            if let Some(decision) = strategy.route(context) {
                return decision;
            }
        }

        RoutingDecision {
            model: context.default_model.clone(),
            metadata: RoutingMetadata {
                source: "fallback".to_string(),
                reasoning: "All strategies failed, using fallback model".to_string(),
                latency_ms: 0,
            },
        }
    }

    pub fn add_strategy(&mut self, strategy: Arc<dyn RoutingStrategy>) {
        self.strategies.push(strategy);
    }
}

impl Default for RoutingEngine {
    fn default() -> Self {
        Self::new()
    }
}
