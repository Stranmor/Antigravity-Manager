use crate::proxy::mappers::claude::models::{ClaudeRequest, MessageContent};
use crate::proxy::mappers::openai::models::{OpenAIContent, OpenAIRequest};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub struct SessionManager;

impl SessionManager {
    /// Extract session ID with priority:
    /// 1. metadata.user_id (if valid format from SDK/CLI)
    /// 2. X-Session-Id header (if provided by client)  
    /// 3. Content-based fingerprint (fallback)
    pub fn extract_session_id(request: &ClaudeRequest) -> String {
        if let Some(id) = Self::extract_from_metadata(request.metadata.as_ref()) {
            return id;
        }

        Self::generate_content_fingerprint(request)
    }

    fn extract_from_metadata(
        metadata: Option<&crate::proxy::mappers::claude::models::Metadata>,
    ) -> Option<String> {
        let metadata = metadata?;
        let user_id = metadata.user_id.as_ref()?;

        if user_id.is_empty() {
            return None;
        }

        // SDK format: "user_{hash}_account_{uuid}_session_{uuid}"
        // This is unique per client session - use it directly
        if user_id.contains("_session_") || user_id.contains("_account_") {
            return Some(user_id.clone());
        }

        // Legacy format "session-{uuid}" - skip, use content fingerprint instead
        if user_id.starts_with("session-") {
            return None;
        }

        // Any other non-empty user_id - trust it
        Some(user_id.clone())
    }

    fn generate_content_fingerprint(request: &ClaudeRequest) -> String {
        let mut hasher = Sha256::new();

        hasher.update(request.model.as_bytes());

        let anchor_text = Self::find_anchor_message(&request.messages);
        if let Some(text) = anchor_text {
            hasher.update(text.as_bytes());
        } else if let Some(last_msg) = request.messages.last() {
            hasher.update(format!("{:?}", last_msg.content).as_bytes());
        }

        let hash = format!("{:x}", hasher.finalize());
        format!("fp-{}", &hash[..16])
    }

    fn find_anchor_message(
        messages: &[crate::proxy::mappers::claude::models::Message],
    ) -> Option<String> {
        for msg in messages {
            if msg.role != "user" {
                continue;
            }

            let text = match &msg.content {
                MessageContent::String(s) => s.clone(),
                MessageContent::Array(blocks) => blocks
                    .iter()
                    .filter_map(|block| match block {
                        crate::proxy::mappers::claude::models::ContentBlock::Text { text } => {
                            Some(text.as_str())
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            };

            let clean = text.trim();
            if clean.len() > 10 && !clean.contains("<system-reminder>") {
                return Some(clean.to_string());
            }
        }
        None
    }

    pub fn extract_openai_session_id(request: &OpenAIRequest) -> String {
        let mut hasher = Sha256::new();
        hasher.update(request.model.as_bytes());

        let anchor = Self::find_openai_anchor(&request.messages);
        if let Some(text) = anchor {
            hasher.update(text.as_bytes());
        } else if let Some(last_msg) = request.messages.last() {
            hasher.update(format!("{:?}", last_msg.content).as_bytes());
        }

        let hash = format!("{:x}", hasher.finalize());
        format!("fp-{}", &hash[..16])
    }

    fn find_openai_anchor(
        messages: &[crate::proxy::mappers::openai::models::OpenAIMessage],
    ) -> Option<String> {
        for msg in messages {
            if msg.role != "user" {
                continue;
            }
            if let Some(content) = &msg.content {
                let text = match content {
                    OpenAIContent::String(s) => s.clone(),
                    OpenAIContent::Array(blocks) => blocks
                        .iter()
                        .filter_map(|block| match block {
                            crate::proxy::mappers::openai::models::OpenAIContentBlock::Text {
                                text,
                            } => Some(text.as_str()),
                            crate::proxy::mappers::openai::models::OpenAIContentBlock::ImageUrl { .. } => None,
                        })
                        .collect::<Vec<_>>()
                        .join(" "),
                };

                let clean = text.trim();
                if clean.len() > 10 && !clean.contains("<system-reminder>") {
                    return Some(clean.to_string());
                }
            }
        }
        None
    }

    pub fn extract_gemini_session_id(request: &Value, model_name: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(model_name.as_bytes());

        let anchor = Self::find_gemini_anchor(request);
        if let Some(text) = anchor {
            hasher.update(text.as_bytes());
        } else {
            hasher.update(request.to_string().as_bytes());
        }

        let hash = format!("{:x}", hasher.finalize());
        format!("fp-{}", &hash[..16])
    }

    fn find_gemini_anchor(request: &Value) -> Option<String> {
        let contents = request.get("contents")?.as_array()?;

        for content in contents {
            if content.get("role").and_then(|v| v.as_str()) != Some("user") {
                continue;
            }

            let parts = content.get("parts")?.as_array()?;
            let text: String = parts
                .iter()
                .filter_map(|p| p.get("text").and_then(|v| v.as_str()))
                .collect::<Vec<_>>()
                .join(" ");

            let clean = text.trim();
            if clean.len() > 10 && !clean.contains("<system-reminder>") {
                return Some(clean.to_string());
            }
        }
        None
    }
}
