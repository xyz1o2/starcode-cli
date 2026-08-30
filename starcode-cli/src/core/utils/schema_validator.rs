use jsonschema::{Draft, JSONSchema};
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

        let compiled_schema = JSONSchema::options()
            .with_draft(Draft::Draft7)
            .compile(schema)
            .map_err(|e| format!("Failed to compile schema: {}", e))?;

        let result = compiled_schema.validate(data);

        if let Err(errors) = result {
            let error_messages: Vec<String> = errors.map(|e| format!("params: {}", e)).collect();
            return Err(error_messages.join(", "));
        }

        Ok(())
    }
}
