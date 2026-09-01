use serde_json::{json, Value};

/// Configuration for the normalization process
#[derive(Debug, Clone)]
pub struct NormalizationConfig {
    /// Target size in bytes (approximate)
    pub target_size: usize,
    /// Minimum string length to preserve (never truncate below this)
    pub min_string_len: usize,
    /// Minimum array length to preserve
    pub min_array_len: usize,
    /// Initial string truncation limit
    pub initial_string_limit: usize,
    /// Initial array truncation limit
    pub initial_array_limit: usize,
}

impl Default for NormalizationConfig {
    fn default() -> Self {
        Self {
            target_size: 80 * 1024, // 80KB default target (increased to reduce truncation)
            min_string_len: 100,
            min_array_len: 5,
            initial_string_limit: 1000,
            initial_array_limit: 50,
        }
    }
}

/// Normalize a JSON value to fit within a target size budget.
/// This iteratively reduces string lengths and array sizes until the structure fits
/// or we hit the minimum limits.
pub fn normalize_to_size(value: Value, config: Option<NormalizationConfig>) -> Value {
    let config = config.unwrap_or_default();
    let mut current_value = value;

    // Quick check: if it already fits, return
    if estimate_size(&current_value) <= config.target_size {
        return current_value;
    }

    let mut string_limit = config.initial_string_limit;
    let mut array_limit = config.initial_array_limit;

    // Iteratively reduce limits until we fit or hit bottom
    loop {
        let mut changed = false;
        current_value = truncate_value(current_value, string_limit, array_limit, &mut changed);

        let size = estimate_size(&current_value);
        if size <= config.target_size {
            break;
        }

        // If we can't reduce further, stop
        if string_limit <= config.min_string_len && array_limit <= config.min_array_len {
            break;
        }

        // Reduce limits for next iteration (decay factor 0.7)
        let next_string = (string_limit as f32 * 0.7) as usize;
        let next_array = (array_limit as f32 * 0.7) as usize;

        string_limit = next_string.max(config.min_string_len);
        array_limit = next_array.max(config.min_array_len);

        // If nothing changed in the last pass but we are still over budget,
        // and we haven't hit min limits, force a reduction on limits to try again.
        // If we hit min limits and nothing changed, we are stuck, so break.
        if !changed && string_limit == config.min_string_len && array_limit == config.min_array_len
        {
            break;
        }
    }

    current_value
}

fn estimate_size(v: &Value) -> usize {
    // fast approximation of JSON size
    // serde_json::to_vec(v).map(|v| v.len()).unwrap_or(0)
    // improved: avoid allocation if possible, but to_string is easiest for exact JSON size
    serde_json::to_string(v).map(|s| s.len()).unwrap_or(0)
}

fn truncate_value(v: Value, string_limit: usize, array_limit: usize, changed: &mut bool) -> Value {
    match v {
        Value::String(s) => {
            if s.len() > string_limit {
                *changed = true;
                // Keep start and end for context if possible
                let half = string_limit / 2;
                if half > 10 {
                    let char_count = s.chars().count();
                    if char_count > string_limit {
                        let start: String = s.chars().take(half).collect();
                        let end: String = s.chars().skip(char_count - half).collect();
                        json!(format!(
                            "{} ... [truncated {} chars] ... {}",
                            start,
                            char_count - string_limit,
                            end
                        ))
                    } else {
                        // Edge case where byte len > limit but char count <= limit (e.g. multi-byte chars)
                        // Just keep it or truncate by bytes if strictly needed.
                        // For safety, if byte len is massive, we should still truncate.
                        // Let's use simple byte slicing if char slicing is complex,
                        // but safe slicing is better.
                        // Fallback to simple truncation
                        json!(format!(
                            "{} ... [truncated]",
                            s.chars().take(string_limit).collect::<String>()
                        ))
                    }
                } else {
                    json!(format!(
                        "{} ... [truncated]",
                        s.chars().take(string_limit).collect::<String>()
                    ))
                }
            } else {
                Value::String(s)
            }
        }
        Value::Array(mut arr) => {
            let original_len = arr.len();
            let mut _arr_changed = false;

            // Truncate elements recursively
            for item in arr.iter_mut() {
                // We clone here to avoid moving out of borrow, optimizations possible
                let old_item = item.clone();
                let new_item = truncate_value(old_item, string_limit, array_limit, changed);
                if *item != new_item {
                    *item = new_item;
                    _arr_changed = true;
                }
            }

            // Truncate array length
            if arr.len() > array_limit {
                *changed = true;
                arr.truncate(array_limit);
                arr.push(json!(format!(
                    "... [{} items truncated]",
                    original_len - array_limit
                )));
            }

            Value::Array(arr)
        }
        Value::Object(mut map) => {
            for (_, v) in map.iter_mut() {
                let old_v = v.clone();
                let new_v = truncate_value(old_v, string_limit, array_limit, changed);
                if *v != new_v {
                    *v = new_v;
                }
            }
            Value::Object(map)
        }
        _ => v,
    }
}
