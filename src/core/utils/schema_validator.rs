use jsonschema::{Draft, Validator};
use serde_json::Value;

pub struct SchemaValidator;

impl SchemaValidator {
    pub fn validate(schema: Option<&Value>, data: &Value) -> Result<(), String> {
        let schema = match schema {
            Some(s) => s,
            None => return Ok(()),
        };

        if !data.is_object() {
            return Err("Value of params must be an object".to_string());
        }

        // jsonschema 0.58：JSONSchema 更名为 Validator，compile() 改为 build()，
        // 且类型上多了一个默认泛型参数 F: Json。
        let compiled_schema: Validator =
            jsonschema::options()
                .with_draft(Draft::Draft7)
                .build(schema)
                .map_err(|e| format!("Failed to compile schema: {}", e))?;

        // 0.58 起 validate() 只回首个错误；要保留旧版「收集全部错误再拼接」的
        // 行为与输出格式，用 iter_errors()。
        let error_messages: Vec<String> = compiled_schema
            .iter_errors(data)
            .map(|e| format!("params: {}", e))
            .collect();
        if !error_messages.is_empty() {
            return Err(error_messages.join(", "));
        }

        Ok(())
    }
}
