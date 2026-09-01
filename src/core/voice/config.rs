pub struct VoiceConfig {
    pub enabled: bool,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub language: String,
    pub speech_rate: f32,
    pub volume: f32,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            input_device: None,
            output_device: None,
            language: "en-US".to_string(),
            speech_rate: 1.0,
            volume: 0.8,
        }
    }
}
