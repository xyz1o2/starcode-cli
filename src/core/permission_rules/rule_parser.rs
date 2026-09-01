use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedRule {
    pub tool: String,
    pub action: String,
    pub path: Option<String>,
    pub priority: Option<i32>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RulesFile {
    pub rules: Vec<ParsedRule>,
}

pub struct RuleParser;

impl RuleParser {
    pub fn parse_json(content: &str) -> Result<Vec<ParsedRule>, String> {
        let file: RulesFile = serde_json::from_str(content)
            .map_err(|e| format!("Failed to parse JSON rules: {}", e))?;
        Ok(file.rules)
    }

    pub fn parse_toml(content: &str) -> Result<Vec<ParsedRule>, String> {
        let file: RulesFile = toml::from_str(content)
            .map_err(|e| format!("Failed to parse TOML rules: {}", e))?;
        Ok(file.rules)
    }

    pub fn validate_rule(rule: &ParsedRule) -> Result<(), String> {
        if rule.tool.is_empty() {
            return Err("Rule must have a non-empty tool pattern".to_string());
        }

        match rule.action.as_str() {
            "allow" | "deny" => {}
            other if !other.is_empty() => {}
            _ => {
                return Err(format!(
                    "Invalid action '{}'. Must be 'allow', 'deny', or a custom message",
                    rule.action
                ));
            }
        }

        Ok(())
    }

    pub fn validate_rules(rules: &[ParsedRule]) -> Vec<String> {
        let mut errors = Vec::new();
        for (i, rule) in rules.iter().enumerate() {
            if let Err(e) = Self::validate_rule(rule) {
                errors.push(format!("Rule {}: {}", i, e));
            }
        }
        errors
    }

    pub fn to_json(rules: &[ParsedRule]) -> Result<String, String> {
        let file = RulesFile {
            rules: rules.to_vec(),
        };
        serde_json::to_string_pretty(&file).map_err(|e| format!("Failed to serialize rules: {}", e))
    }

    pub fn to_toml(rules: &[ParsedRule]) -> Result<String, String> {
        let file = RulesFile {
            rules: rules.to_vec(),
        };
        toml::to_string_pretty(&file).map_err(|e| format!("Failed to serialize rules: {}", e))
    }
}

 