pub const BACKGROUND_MODEL_LITE: &str = "gemini-2.5-flash-lite";
pub const BACKGROUND_MODEL_STANDARD: &str = "gemini-2.5-flash";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BackgroundTaskType {
    TitleGeneration,
    SimpleSummary,
    ContextCompression,
    PromptSuggestion,
    SystemMessage,
    EnvironmentProbe,
}

const TITLE_KEYWORDS: &[&str] = &[
    "write a 5-10 word title",
    "Please write a 5-10 word title",
    "Respond with the title",
    "Generate a title for",
    "Create a brief title",
    "title for the conversation",
    "conversation title",
    "生成标题",
    "为对话起个标题",
];

const SUMMARY_KEYWORDS: &[&str] = &[
    "Summarize this coding conversation",
    "Summarize the conversation",
    "Concise summary",
    "in under 50 characters",
    "compress the context",
    "Provide a concise summary",
    "condense the previous messages",
    "shorten the conversation history",
    "extract key points from",
];

const SUGGESTION_KEYWORDS: &[&str] = &[
    "prompt suggestion generator",
    "suggest next prompts",
    "what should I ask next",
    "generate follow-up questions",
    "recommend next steps",
    "possible next actions",
];

const SYSTEM_KEYWORDS: &[&str] = &[
    "Warmup",
    "<system-reminder>",
    "This is a system message",
];

const PROBE_KEYWORDS: &[&str] = &[
    "check current directory",
    "list available tools",
    "verify environment",
    "test connection",
];

pub fn detect_from_text(text: &str) -> Option<BackgroundTaskType> {
    if text.len() > 800 {
        return None;
    }

    let preview: String = text.chars().take(500).collect();

    if matches_keywords(&preview, SYSTEM_KEYWORDS) {
        return Some(BackgroundTaskType::SystemMessage);
    }

    if matches_keywords(&preview, TITLE_KEYWORDS) {
        return Some(BackgroundTaskType::TitleGeneration);
    }

    if matches_keywords(&preview, SUMMARY_KEYWORDS) {
        if preview.contains("in under 50 characters") {
            return Some(BackgroundTaskType::SimpleSummary);
        }
        return Some(BackgroundTaskType::ContextCompression);
    }

    if matches_keywords(&preview, SUGGESTION_KEYWORDS) {
        return Some(BackgroundTaskType::PromptSuggestion);
    }

    if matches_keywords(&preview, PROBE_KEYWORDS) {
        return Some(BackgroundTaskType::EnvironmentProbe);
    }

    None
}

fn matches_keywords(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|kw| text.contains(kw))
}

pub fn select_model(task_type: BackgroundTaskType) -> &'static str {
    match task_type {
        BackgroundTaskType::TitleGeneration
        | BackgroundTaskType::SimpleSummary
        | BackgroundTaskType::SystemMessage
        | BackgroundTaskType::PromptSuggestion
        | BackgroundTaskType::EnvironmentProbe => BACKGROUND_MODEL_LITE,
        BackgroundTaskType::ContextCompression => BACKGROUND_MODEL_STANDARD,
    }
}
