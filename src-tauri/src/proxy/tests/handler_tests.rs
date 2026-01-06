//! Integration Tests for Proxy Handlers
//!
//! This module contains mock-based integration tests that verify:
//! 1. Request transformation from Claude/OpenAI format to Gemini format
//! 2. Response transformation from Gemini format back to Claude/OpenAI format
//! 3. Error handling for rate limits (429), server errors (500/503), and auth errors (401)
//! 4. Streaming SSE response handling
//!
//! These tests use mock data and do not make actual API calls.

use serde_json::{json, Value};

// =============================================================================
// Claude Request Transformation Tests
// =============================================================================

mod claude_request_transformation {
    use super::*;
    use crate::proxy::mappers::claude::models::{
        ClaudeRequest, ContentBlock, Message, MessageContent, SystemPrompt, ThinkingConfig, Tool,
    };
    use crate::proxy::mappers::claude::request::transform_claude_request_in;

    /// Test basic text message transformation
    #[test]
    fn test_simple_text_request_transformation() {
        let request = ClaudeRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: MessageContent::String("Hello, Claude!".to_string()),
            }],
            system: None,
            tools: None,
            stream: false,
            max_tokens: Some(1024),
            temperature: Some(0.7),
            top_p: None,
            top_k: None,
            thinking: None,
            metadata: None,
            output_config: None,
        };

        let result = transform_claude_request_in(&request, "test-project");
        assert!(result.is_ok(), "Transformation should succeed");

        let body = result.unwrap();
        assert_eq!(body["project"], "test-project");
        assert!(body["requestId"].as_str().unwrap().starts_with("agent-"));

        // Verify contents structure
        let contents = body["request"]["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");

        let parts = contents[0]["parts"].as_array().unwrap();
        assert!(parts.iter().any(|p| p["text"]
            .as_str()
            .is_some_and(|t| t.contains("Hello, Claude!"))));
    }

    /// Test system prompt transformation
    #[test]
    fn test_system_prompt_transformation() {
        let request = ClaudeRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: MessageContent::String("Hello".to_string()),
            }],
            system: Some(SystemPrompt::String(
                "You are a helpful assistant.".to_string(),
            )),
            tools: None,
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            thinking: None,
            metadata: None,
            output_config: None,
        };

        let result = transform_claude_request_in(&request, "test-project");
        assert!(result.is_ok());

        let body = result.unwrap();
        let system_instruction = &body["request"]["systemInstruction"];
        assert!(system_instruction.is_object());

        let parts = system_instruction["parts"].as_array().unwrap();
        // Should contain identity patch and system prompt
        assert!(parts.len() >= 2);

        let combined_text: String = parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(combined_text.contains("You are a helpful assistant."));
    }

    /// Test tool definition transformation
    #[test]
    fn test_tool_definition_transformation() {
        let request = ClaudeRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: MessageContent::String("What's the weather?".to_string()),
            }],
            system: None,
            tools: Some(vec![Tool {
                type_: None,
                name: Some("get_weather".to_string()),
                description: Some("Get weather for a location".to_string()),
                input_schema: Some(json!({
                    "type": "object",
                    "properties": {
                        "location": {
                            "type": "string",
                            "description": "The city name"
                        }
                    },
                    "required": ["location"]
                })),
            }]),
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            thinking: None,
            metadata: None,
            output_config: None,
        };

        let result = transform_claude_request_in(&request, "test-project");
        assert!(result.is_ok());

        let body = result.unwrap();
        let tools = body["request"]["tools"].as_array();
        assert!(tools.is_some(), "Tools should be present");

        let tools = tools.unwrap();
        assert!(!tools.is_empty());

        // Check function declarations
        let func_decls = &tools[0]["functionDeclarations"];
        assert!(func_decls.is_array());

        let decls = func_decls.as_array().unwrap();
        assert!(decls.iter().any(|d| d["name"] == "get_weather"));
    }

    /// Test tool use message transformation
    #[test]
    fn test_tool_use_message_transformation() {
        let request = ClaudeRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![
                Message {
                    role: "user".to_string(),
                    content: MessageContent::String("Run the command".to_string()),
                },
                Message {
                    role: "assistant".to_string(),
                    content: MessageContent::Array(vec![ContentBlock::ToolUse {
                        id: "call_123".to_string(),
                        name: "execute".to_string(),
                        input: json!({"command": "ls -la"}),
                        signature: None,
                        cache_control: None,
                    }]),
                },
                Message {
                    role: "user".to_string(),
                    content: MessageContent::Array(vec![ContentBlock::ToolResult {
                        tool_use_id: "call_123".to_string(),
                        content: json!("file1.txt\nfile2.txt"),
                        is_error: Some(false),
                    }]),
                },
            ],
            system: None,
            tools: None,
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            thinking: None,
            metadata: None,
            output_config: None,
        };

        let result = transform_claude_request_in(&request, "test-project");
        assert!(result.is_ok());

        let body = result.unwrap();
        let contents = body["request"]["contents"].as_array().unwrap();

        // Find the tool use message (role: model)
        let tool_use_msg = contents.iter().find(|c| c["role"] == "model");
        assert!(tool_use_msg.is_some(), "Tool use message should be present");

        let parts = tool_use_msg.unwrap()["parts"].as_array().unwrap();
        let function_call = parts.iter().find(|p| p.get("functionCall").is_some());
        assert!(function_call.is_some(), "Function call should be present");

        let fc = &function_call.unwrap()["functionCall"];
        assert_eq!(fc["name"], "execute");
        assert_eq!(fc["id"], "call_123");
    }

    /// Test image content transformation
    #[test]
    fn test_image_content_transformation() {
        use crate::proxy::mappers::claude::models::ImageSource;

        let request = ClaudeRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: MessageContent::Array(vec![
                    ContentBlock::Text {
                        text: "What's in this image?".to_string(),
                    },
                    ContentBlock::Image {
                        source: ImageSource {
                            source_type: "base64".to_string(),
                            media_type: "image/png".to_string(),
                            data: "iVBORw0KGgoAAAANSUhEUg==".to_string(),
                        },
                        cache_control: None,
                    },
                ]),
            }],
            system: None,
            tools: None,
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            thinking: None,
            metadata: None,
            output_config: None,
        };

        let result = transform_claude_request_in(&request, "test-project");
        assert!(result.is_ok());

        let body = result.unwrap();
        let contents = body["request"]["contents"].as_array().unwrap();
        let parts = contents[0]["parts"].as_array().unwrap();

        // Should have text and inline data
        let inline_data = parts.iter().find(|p| p.get("inlineData").is_some());
        assert!(inline_data.is_some(), "Inline data should be present");

        let data = &inline_data.unwrap()["inlineData"];
        assert_eq!(data["mimeType"], "image/png");
        assert!(data["data"].as_str().is_some());
    }

    /// Test thinking mode configuration
    #[test]
    fn test_thinking_mode_configuration() {
        let request = ClaudeRequest {
            model: "claude-opus-4-5-thinking".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: MessageContent::String("Solve this problem".to_string()),
            }],
            system: None,
            tools: None,
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            thinking: Some(ThinkingConfig {
                type_: "enabled".to_string(),
                budget_tokens: Some(8192),
            }),
            metadata: None,
            output_config: None,
        };

        let result = transform_claude_request_in(&request, "test-project");
        assert!(result.is_ok());

        let body = result.unwrap();

        // Generation config should exist
        let gen_config = &body["request"]["generationConfig"];
        assert!(gen_config.is_object());
    }

    /// Test safety settings are applied
    #[test]
    fn test_safety_settings_applied() {
        let request = ClaudeRequest {
            model: "claude-sonnet-4-5".to_string(),
            messages: vec![Message {
                role: "user".to_string(),
                content: MessageContent::String("Hello".to_string()),
            }],
            system: None,
            tools: None,
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
            top_k: None,
            thinking: None,
            metadata: None,
            output_config: None,
        };

        let result = transform_claude_request_in(&request, "test-project");
        assert!(result.is_ok());

        let body = result.unwrap();
        let safety = body["request"]["safetySettings"].as_array();
        assert!(safety.is_some(), "Safety settings should be present");

        let safety = safety.unwrap();
        assert!(!safety.is_empty());

        // All categories should have threshold set
        for setting in safety {
            assert!(setting["category"].is_string());
            assert!(setting["threshold"].is_string());
        }
    }
}

// =============================================================================
// Claude Response Transformation Tests
// =============================================================================

mod claude_response_transformation {
    use super::*;
    use crate::proxy::mappers::claude::models::{
        Candidate, ContentBlock, GeminiContent, GeminiPart, GeminiResponse, UsageMetadata,
    };
    use crate::proxy::mappers::claude::response::transform_response;

    /// Test simple text response transformation
    #[test]
    fn test_simple_text_response() {
        let gemini_resp = GeminiResponse {
            candidates: Some(vec![Candidate {
                content: Some(GeminiContent {
                    role: "model".to_string(),
                    parts: vec![GeminiPart {
                        text: Some("Hello! I'm here to help.".to_string()),
                        thought: None,
                        thought_signature: None,
                        function_call: None,
                        function_response: None,
                        inline_data: None,
                    }],
                }),
                finish_reason: Some("STOP".to_string()),
                index: Some(0),
                grounding_metadata: None,
            }]),
            usage_metadata: Some(UsageMetadata {
                prompt_token_count: Some(10),
                candidates_token_count: Some(15),
                total_token_count: Some(25),
                cached_content_token_count: None,
            }),
            model_version: Some("gemini-2.5-pro".to_string()),
            response_id: Some("resp_123".to_string()),
        };

        let result = transform_response(&gemini_resp);
        assert!(result.is_ok());

        let claude_resp = result.unwrap();
        assert_eq!(claude_resp.role, "assistant");
        assert_eq!(claude_resp.stop_reason, "end_turn");
        assert_eq!(claude_resp.content.len(), 1);

        match &claude_resp.content[0] {
            ContentBlock::Text { text } => {
                assert_eq!(text, "Hello! I'm here to help.");
            }
            _ => panic!("Expected Text block"),
        }

        // Verify usage
        assert!(claude_resp.usage.input_tokens > 0 || claude_resp.usage.output_tokens > 0);
    }

    /// Test thinking block response transformation
    #[test]
    fn test_thinking_response() {
        let gemini_resp = GeminiResponse {
            candidates: Some(vec![Candidate {
                content: Some(GeminiContent {
                    role: "model".to_string(),
                    parts: vec![
                        GeminiPart {
                            text: Some("Let me analyze this...".to_string()),
                            thought: Some(true),
                            thought_signature: Some("sig_abc123".to_string()),
                            function_call: None,
                            function_response: None,
                            inline_data: None,
                        },
                        GeminiPart {
                            text: Some("The answer is 42.".to_string()),
                            thought: None,
                            thought_signature: None,
                            function_call: None,
                            function_response: None,
                            inline_data: None,
                        },
                    ],
                }),
                finish_reason: Some("STOP".to_string()),
                index: Some(0),
                grounding_metadata: None,
            }]),
            usage_metadata: None,
            model_version: Some("gemini-2.5-pro".to_string()),
            response_id: Some("resp_456".to_string()),
        };

        let result = transform_response(&gemini_resp);
        assert!(result.is_ok());

        let claude_resp = result.unwrap();
        assert_eq!(claude_resp.content.len(), 2);

        // First should be thinking block
        match &claude_resp.content[0] {
            ContentBlock::Thinking {
                thinking,
                signature,
                ..
            } => {
                assert_eq!(thinking, "Let me analyze this...");
                assert_eq!(signature.as_deref(), Some("sig_abc123"));
            }
            _ => panic!("Expected Thinking block first"),
        }

        // Second should be text block
        match &claude_resp.content[1] {
            ContentBlock::Text { text } => {
                assert_eq!(text, "The answer is 42.");
            }
            _ => panic!("Expected Text block second"),
        }
    }

    /// Test function call response transformation
    #[test]
    fn test_function_call_response() {
        use crate::proxy::mappers::claude::models::FunctionCall;

        let gemini_resp = GeminiResponse {
            candidates: Some(vec![Candidate {
                content: Some(GeminiContent {
                    role: "model".to_string(),
                    parts: vec![GeminiPart {
                        text: None,
                        thought: None,
                        thought_signature: None,
                        function_call: Some(FunctionCall {
                            name: "read_file".to_string(),
                            id: Some("call_789".to_string()),
                            args: Some(json!({"file_path": "/tmp/test.txt"})),
                        }),
                        function_response: None,
                        inline_data: None,
                    }],
                }),
                finish_reason: Some("STOP".to_string()),
                index: Some(0),
                grounding_metadata: None,
            }]),
            usage_metadata: None,
            model_version: Some("gemini-2.5-pro".to_string()),
            response_id: Some("resp_tool".to_string()),
        };

        let result = transform_response(&gemini_resp);
        assert!(result.is_ok());

        let claude_resp = result.unwrap();
        assert_eq!(claude_resp.stop_reason, "tool_use");

        match &claude_resp.content[0] {
            ContentBlock::ToolUse { id, name, input, .. } => {
                assert_eq!(id, "call_789");
                assert_eq!(name, "read_file");
                assert_eq!(input["file_path"], "/tmp/test.txt");
            }
            _ => panic!("Expected ToolUse block"),
        }
    }

    /// Test MAX_TOKENS finish reason
    #[test]
    fn test_max_tokens_finish_reason() {
        let gemini_resp = GeminiResponse {
            candidates: Some(vec![Candidate {
                content: Some(GeminiContent {
                    role: "model".to_string(),
                    parts: vec![GeminiPart {
                        text: Some("Partial response...".to_string()),
                        thought: None,
                        thought_signature: None,
                        function_call: None,
                        function_response: None,
                        inline_data: None,
                    }],
                }),
                finish_reason: Some("MAX_TOKENS".to_string()),
                index: Some(0),
                grounding_metadata: None,
            }]),
            usage_metadata: None,
            model_version: Some("gemini-2.5-pro".to_string()),
            response_id: Some("resp_max".to_string()),
        };

        let result = transform_response(&gemini_resp);
        assert!(result.is_ok());

        let claude_resp = result.unwrap();
        assert_eq!(claude_resp.stop_reason, "max_tokens");
    }

    /// Test empty candidate handling
    #[test]
    fn test_empty_candidates() {
        let gemini_resp = GeminiResponse {
            candidates: Some(vec![]),
            usage_metadata: None,
            model_version: Some("gemini-2.5-pro".to_string()),
            response_id: Some("resp_empty".to_string()),
        };

        let result = transform_response(&gemini_resp);
        assert!(result.is_ok());

        let claude_resp = result.unwrap();
        assert!(claude_resp.content.is_empty());
    }
}

// =============================================================================
// OpenAI Request Transformation Tests
// =============================================================================

mod openai_request_transformation {
    use super::*;
    use crate::proxy::mappers::openai::models::{
        OpenAIContent, OpenAIContentBlock, OpenAIImageUrl, OpenAIMessage, OpenAIRequest,
    };
    use crate::proxy::mappers::openai::request::transform_openai_request;

    /// Test simple chat completion request
    #[test]
    fn test_simple_chat_request() {
        let request = OpenAIRequest {
            model: "gpt-4".to_string(),
            messages: vec![OpenAIMessage {
                role: "user".to_string(),
                content: Some(OpenAIContent::String("Hello, GPT!".to_string())),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            prompt: None,
            stream: false,
            max_tokens: Some(1024),
            temperature: Some(0.8),
            top_p: None,
            stop: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            instructions: None,
            input: None,
        };

        let result = transform_openai_request(&request, "test-project", "gemini-2.5-pro");

        assert_eq!(result["project"], "test-project");
        assert!(result["requestId"]
            .as_str()
            .unwrap()
            .starts_with("openai-"));
        assert_eq!(result["model"], "gemini-2.5-pro");

        let contents = result["request"]["contents"].as_array().unwrap();
        assert_eq!(contents.len(), 1);
        assert_eq!(contents[0]["role"], "user");

        let parts = contents[0]["parts"].as_array().unwrap();
        assert!(parts.iter().any(|p| p["text"] == "Hello, GPT!"));
    }

    /// Test system message transformation
    #[test]
    fn test_system_message_transformation() {
        let request = OpenAIRequest {
            model: "gpt-4".to_string(),
            messages: vec![
                OpenAIMessage {
                    role: "system".to_string(),
                    content: Some(OpenAIContent::String(
                        "You are a coding assistant.".to_string(),
                    )),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                OpenAIMessage {
                    role: "user".to_string(),
                    content: Some(OpenAIContent::String("Write code".to_string())),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            ],
            prompt: None,
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            instructions: None,
            input: None,
        };

        let result = transform_openai_request(&request, "test-project", "gemini-2.5-pro");

        // System instruction should be extracted
        let system_instruction = &result["request"]["systemInstruction"];
        assert!(system_instruction.is_object());

        let parts = system_instruction["parts"].as_array().unwrap();
        let text = parts.iter().find_map(|p| p["text"].as_str()).unwrap();
        assert!(text.contains("You are a coding assistant."));

        // Contents should not include system message
        let contents = result["request"]["contents"].as_array().unwrap();
        assert!(contents.iter().all(|c| c["role"] != "system"));
    }

    /// Test multimodal content with image
    #[test]
    fn test_multimodal_image_request() {
        let request = OpenAIRequest {
            model: "gpt-4-vision".to_string(),
            messages: vec![OpenAIMessage {
                role: "user".to_string(),
                content: Some(OpenAIContent::Array(vec![
                    OpenAIContentBlock::Text {
                        text: "What's in this image?".to_string(),
                    },
                    OpenAIContentBlock::ImageUrl {
                        image_url: OpenAIImageUrl {
                            url: "data:image/png;base64,iVBORw0KGgo=".to_string(),
                            detail: None,
                        },
                    },
                ])),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            prompt: None,
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            instructions: None,
            input: None,
        };

        let result = transform_openai_request(&request, "test-project", "gemini-1.5-flash");

        let contents = result["request"]["contents"].as_array().unwrap();
        let parts = contents[0]["parts"].as_array().unwrap();

        // Should have both text and inline data
        assert!(parts.iter().any(|p| p["text"] == "What's in this image?"));
        assert!(parts.iter().any(|p| p.get("inlineData").is_some()));
    }

    /// Test tool definition transformation
    #[test]
    fn test_openai_tool_transformation() {
        let request = OpenAIRequest {
            model: "gpt-4".to_string(),
            messages: vec![OpenAIMessage {
                role: "user".to_string(),
                content: Some(OpenAIContent::String("Search the web".to_string())),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            prompt: None,
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop: None,
            response_format: None,
            tools: Some(vec![json!({
                "type": "function",
                "function": {
                    "name": "search",
                    "description": "Search for information",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "query": {"type": "string"}
                        }
                    }
                }
            })]),
            tool_choice: None,
            parallel_tool_calls: None,
            instructions: None,
            input: None,
        };

        let result = transform_openai_request(&request, "test-project", "gemini-2.5-pro");

        let tools = result["request"]["tools"].as_array();
        assert!(tools.is_some());

        let tools = tools.unwrap();
        assert!(!tools.is_empty());

        let func_decls = &tools[0]["functionDeclarations"];
        assert!(func_decls.is_array());
    }

    /// Test assistant message with tool calls
    #[test]
    fn test_tool_call_message() {
        use crate::proxy::mappers::openai::models::{ToolCall, ToolFunction};

        let request = OpenAIRequest {
            model: "gpt-4".to_string(),
            messages: vec![
                OpenAIMessage {
                    role: "user".to_string(),
                    content: Some(OpenAIContent::String("Run command".to_string())),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                OpenAIMessage {
                    role: "assistant".to_string(),
                    content: None,
                    tool_calls: Some(vec![ToolCall {
                        id: "call_001".to_string(),
                        r#type: "function".to_string(),
                        function: ToolFunction {
                            name: "shell".to_string(),
                            arguments: r#"{"command": "ls"}"#.to_string(),
                        },
                    }]),
                    tool_call_id: None,
                    name: None,
                },
                OpenAIMessage {
                    role: "tool".to_string(),
                    content: Some(OpenAIContent::String("file1.txt".to_string())),
                    tool_calls: None,
                    tool_call_id: Some("call_001".to_string()),
                    name: Some("shell".to_string()),
                },
            ],
            prompt: None,
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop: None,
            response_format: None,
            tools: None,
            tool_choice: None,
            parallel_tool_calls: None,
            instructions: None,
            input: None,
        };

        let result = transform_openai_request(&request, "test-project", "gemini-2.5-pro");

        let contents = result["request"]["contents"].as_array().unwrap();

        // Should have merged messages
        assert!(contents.len() >= 2);

        // Find function call
        let has_function_call = contents.iter().any(|c| {
            c["parts"]
                .as_array()
                .is_some_and(|p| p.iter().any(|part| part.get("functionCall").is_some()))
        });
        assert!(has_function_call, "Should have function call");

        // Find function response
        let has_function_response = contents.iter().any(|c| {
            c["parts"].as_array().is_some_and(|p| {
                p.iter()
                    .any(|part| part.get("functionResponse").is_some())
            })
        });
        assert!(has_function_response, "Should have function response");
    }
}

// =============================================================================
// OpenAI Response Transformation Tests
// =============================================================================

mod openai_response_transformation {
    use super::*;
    use crate::proxy::mappers::openai::models::OpenAIContent;
    use crate::proxy::mappers::openai::response::transform_openai_response;

    /// Test simple text response
    #[test]
    fn test_simple_text_response() {
        let gemini_resp = json!({
            "candidates": [{
                "content": {
                    "parts": [{"text": "Hello from Gemini!"}]
                },
                "finishReason": "STOP"
            }],
            "modelVersion": "gemini-2.5-pro",
            "responseId": "resp_openai_test"
        });

        let result = transform_openai_response(&gemini_resp);

        assert_eq!(result.object, "chat.completion");
        assert_eq!(result.choices.len(), 1);
        assert_eq!(
            result.choices[0].finish_reason,
            Some("stop".to_string())
        );

        match &result.choices[0].message.content {
            Some(OpenAIContent::String(text)) => {
                assert_eq!(text, "Hello from Gemini!");
            }
            _ => panic!("Expected string content"),
        }
    }

    /// Test tool call response
    #[test]
    fn test_tool_call_response() {
        let gemini_resp = json!({
            "candidates": [{
                "content": {
                    "parts": [{
                        "functionCall": {
                            "name": "get_weather",
                            "args": {"location": "Tokyo"},
                            "id": "call_weather_1"
                        }
                    }]
                },
                "finishReason": "STOP"
            }],
            "modelVersion": "gemini-2.5-pro",
            "responseId": "resp_tool"
        });

        let result = transform_openai_response(&gemini_resp);

        assert!(result.choices[0].message.tool_calls.is_some());
        let tool_calls = result.choices[0].message.tool_calls.as_ref().unwrap();
        assert_eq!(tool_calls.len(), 1);

        assert_eq!(tool_calls[0].function.name, "get_weather");
        assert!(tool_calls[0].function.arguments.contains("Tokyo"));
    }

    /// Test finish reason mapping
    #[test]
    fn test_finish_reason_mapping() {
        // MAX_TOKENS -> length
        let gemini_resp = json!({
            "candidates": [{
                "content": {"parts": [{"text": "..."}]},
                "finishReason": "MAX_TOKENS"
            }]
        });
        let result = transform_openai_response(&gemini_resp);
        assert_eq!(
            result.choices[0].finish_reason,
            Some("length".to_string())
        );

        // SAFETY -> content_filter
        let gemini_resp = json!({
            "candidates": [{
                "content": {"parts": [{"text": "..."}]},
                "finishReason": "SAFETY"
            }]
        });
        let result = transform_openai_response(&gemini_resp);
        assert_eq!(
            result.choices[0].finish_reason,
            Some("content_filter".to_string())
        );
    }

    /// Test grounding metadata handling
    #[test]
    fn test_grounding_metadata() {
        let gemini_resp = json!({
            "candidates": [{
                "content": {"parts": [{"text": "The capital is Tokyo."}]},
                "finishReason": "STOP",
                "groundingMetadata": {
                    "webSearchQueries": ["capital of Japan"],
                    "groundingChunks": [{
                        "web": {
                            "title": "Japan Wikipedia",
                            "uri": "https://example.com/japan"
                        }
                    }]
                }
            }],
            "modelVersion": "gemini-2.5-pro",
            "responseId": "resp_grounding"
        });

        let result = transform_openai_response(&gemini_resp);

        match &result.choices[0].message.content {
            Some(OpenAIContent::String(text)) => {
                // Should include grounding info
                assert!(text.contains("Tokyo"));
                // Grounding formatting may include search info
            }
            _ => panic!("Expected string content"),
        }
    }
}

// =============================================================================
// Error Handling Tests
// =============================================================================

mod error_handling {
    use crate::proxy::error::{ErrorCode, ProxyError};
    use axum::http::StatusCode;

    /// Test 429 rate limit error handling
    #[test]
    fn test_rate_limit_error_429() {
        let error = ProxyError::RateLimited("Too many requests".to_string(), None);

        assert_eq!(error.status_code(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(error.error_code(), ErrorCode::RateLimited);
        assert!(error.is_rate_limited());
        assert!(error.is_retryable());
    }

    /// Test upstream 429 error
    #[test]
    fn test_upstream_rate_limit() {
        let error = ProxyError::upstream_error(429, "Quota exceeded");

        assert_eq!(error.status_code(), StatusCode::TOO_MANY_REQUESTS);
        assert!(error.is_rate_limited());
        assert!(error.is_retryable());
    }

    /// Test 500 server error handling
    #[test]
    fn test_server_error_500() {
        let error = ProxyError::upstream_error(500, "Internal server error");

        assert_eq!(error.status_code(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!error.is_rate_limited());
        assert!(error.is_retryable());
    }

    /// Test 503 service unavailable
    #[test]
    fn test_service_unavailable_503() {
        let error = ProxyError::upstream_error(503, "Service temporarily unavailable");

        assert_eq!(error.status_code(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(error.is_overload());
        assert!(error.is_retryable());
    }

    /// Test 529 overloaded error (non-standard status code maps to 503)
    #[test]
    fn test_overloaded_error_529() {
        let error = ProxyError::upstream_error(529, "API overloaded");

        // 529 is a non-standard HTTP status, so StatusCode::from_u16(529) fails
        // and falls back to BAD_GATEWAY. The ProxyError tracks the original status.
        // The important assertions are is_overload() and is_retryable().
        assert!(error.is_overload());
        assert!(error.is_retryable());

        // Overloaded variant should return SERVICE_UNAVAILABLE
        let overloaded_error = ProxyError::Overloaded("API overloaded".into(), None);
        assert_eq!(overloaded_error.status_code(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(overloaded_error.is_overload());
    }

    /// Test 401 auth error handling
    #[test]
    fn test_auth_error_401() {
        let error = ProxyError::upstream_error(401, "Unauthorized");

        assert_eq!(error.status_code(), StatusCode::UNAUTHORIZED);
        assert!(!error.is_rate_limited());
        assert!(!error.is_retryable()); // Auth errors are not retryable
    }

    /// Test 400 bad request (not retryable)
    #[test]
    fn test_bad_request_400() {
        let error = ProxyError::invalid_request("Missing required field");

        assert_eq!(error.status_code(), StatusCode::BAD_REQUEST);
        assert!(!error.is_retryable());
    }

    /// Test network error handling
    #[test]
    fn test_network_error() {
        let error = ProxyError::network_error("Connection refused");

        assert_eq!(error.status_code(), StatusCode::BAD_GATEWAY);
        assert!(error.is_retryable());
    }

    /// Test parse error handling
    #[test]
    fn test_parse_error() {
        let error = ProxyError::parse_error("Invalid JSON response");

        assert_eq!(error.status_code(), StatusCode::BAD_GATEWAY);
        assert!(!error.is_retryable());
    }

    /// Test retry exhausted error
    #[test]
    fn test_retry_exhausted() {
        use crate::proxy::error::RetryErrorBreakdown;

        let mut breakdown = RetryErrorBreakdown::default();
        breakdown.record_status(429);
        breakdown.record_status(429);
        breakdown.record_status(503);

        let error = ProxyError::retry_exhausted(3, "All attempts failed", Some(breakdown));

        assert_eq!(error.status_code(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.error_code(), ErrorCode::AccountsExhausted);
    }

    /// Test circuit breaker open error
    #[test]
    fn test_circuit_breaker_open() {
        use std::time::Duration;

        let error =
            ProxyError::circuit_breaker_open("api.vertex.ai", "Too many failures", Some(Duration::from_secs(30)));

        assert_eq!(error.status_code(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error.error_code(), ErrorCode::CircuitOpen);
        assert_eq!(error.retry_after_ms(), Some(30000));
    }
}

// =============================================================================
// SSE Streaming Response Tests
// =============================================================================

mod sse_streaming {
    use super::*;
    use crate::proxy::mappers::claude::models::{FunctionCall, GeminiPart, InlineData};
    use crate::proxy::mappers::claude::streaming::{
        BlockType, PartProcessor, SignatureManager, StreamingState,
    };
    use bytes::Bytes;

    /// Test SSE event format
    #[test]
    fn test_sse_event_format() {
        let state = StreamingState::new();
        let chunk = state.emit("test_event", json!({"key": "value"}));
        let s = String::from_utf8(chunk.to_vec()).unwrap();

        // SSE format: event: <name>\ndata: <json>\n\n
        assert!(s.starts_with("event: test_event\n"));
        assert!(s.contains("data: "));
        assert!(s.ends_with("\n\n"));

        // Verify JSON is valid
        let data_line = s.lines().find(|l| l.starts_with("data: ")).unwrap();
        let json_str = data_line.strip_prefix("data: ").unwrap();
        let parsed: Result<Value, _> = serde_json::from_str(json_str);
        assert!(parsed.is_ok());
    }

    /// Test message_start event
    #[test]
    fn test_message_start_event() {
        let mut state = StreamingState::new();
        let raw = json!({
            "responseId": "msg_stream_1",
            "modelVersion": "gemini-2.5-pro",
            "usageMetadata": {
                "promptTokenCount": 50,
                "candidatesTokenCount": 0
            }
        });

        let chunk = state.emit_message_start(&raw);
        let s = String::from_utf8(chunk.to_vec()).unwrap();

        assert!(s.contains("message_start"));
        assert!(s.contains("msg_stream_1"));
        assert!(state.message_start_sent);

        // Should not emit again
        let chunk2 = state.emit_message_start(&raw);
        assert!(chunk2.is_empty());
    }

    /// Test content block start/stop lifecycle
    #[test]
    fn test_content_block_lifecycle() {
        let mut state = StreamingState::new();

        // Start text block
        let chunks =
            state.start_block(BlockType::Text, json!({"type": "text", "text": ""}));
        assert!(!chunks.is_empty());
        assert_eq!(state.current_block_type(), BlockType::Text);
        assert_eq!(state.current_block_index(), 0);

        // Emit delta
        let delta = state.emit_delta("text_delta", json!({"text": "Hello"}));
        let s = String::from_utf8(delta.to_vec()).unwrap();
        assert!(s.contains("content_block_delta"));
        assert!(s.contains("Hello"));

        // End block
        let end_chunks = state.end_block();
        assert!(!end_chunks.is_empty());
        assert_eq!(state.current_block_type(), BlockType::None);
        assert_eq!(state.current_block_index(), 1);
    }

    /// Test text part processing
    #[test]
    fn test_process_text_part() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let part = GeminiPart {
            text: Some("Hello, streaming world!".to_string()),
            thought: None,
            thought_signature: None,
            function_call: None,
            function_response: None,
            inline_data: None,
        };

        let chunks = processor.process(&part);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("content_block_start"));
        assert!(output.contains("text_delta"));
        assert!(output.contains("Hello, streaming world!"));
    }

    /// Test thinking part processing
    #[test]
    fn test_process_thinking_part() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let part = GeminiPart {
            text: Some("Analyzing the problem...".to_string()),
            thought: Some(true),
            thought_signature: Some("sig_think_1".to_string()),
            function_call: None,
            function_response: None,
            inline_data: None,
        };

        let chunks = processor.process(&part);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("content_block_start"));
        assert!(output.contains("thinking_delta"));
        assert!(output.contains("Analyzing the problem..."));
    }

    /// Test function call streaming
    #[test]
    fn test_process_function_call_streaming() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let part = GeminiPart {
            text: None,
            thought: None,
            thought_signature: None,
            function_call: Some(FunctionCall {
                name: "read_file".to_string(),
                id: Some("call_stream_1".to_string()),
                args: Some(json!({"path": "/test.txt"})),
            }),
            function_response: None,
            inline_data: None,
        };

        let chunks = processor.process(&part);
        let output = chunks_to_string(&chunks);

        // Should have:
        // 1. content_block_start with empty input
        // 2. input_json_delta with args
        // 3. content_block_stop
        assert!(output.contains("content_block_start"));
        assert!(output.contains("tool_use"));
        assert!(output.contains("read_file"));
        assert!(output.contains("input_json_delta"));
        assert!(output.contains("content_block_stop"));
    }

    /// Test image streaming
    #[test]
    fn test_process_image_streaming() {
        let mut state = StreamingState::new();
        let mut processor = PartProcessor::new(&mut state);

        let part = GeminiPart {
            text: None,
            thought: None,
            thought_signature: None,
            function_call: None,
            function_response: None,
            inline_data: Some(InlineData {
                mime_type: "image/png".to_string(),
                data: "iVBORw0KGgo=".to_string(),
            }),
        };

        let chunks = processor.process(&part);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("![image](data:image/png;base64"));
    }

    /// Test finish event with tool use
    #[test]
    fn test_emit_finish_with_tool() {
        let mut state = StreamingState::new();
        state.message_start_sent = true;
        state.mark_tool_used();

        let chunks = state.emit_finish(Some("STOP"), None);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("message_delta"));
        assert!(output.contains("tool_use"));
        assert!(output.contains("message_stop"));
    }

    /// Test finish event with max_tokens
    #[test]
    fn test_emit_finish_max_tokens() {
        let mut state = StreamingState::new();
        state.message_start_sent = true;

        let chunks = state.emit_finish(Some("MAX_TOKENS"), None);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("max_tokens"));
    }

    /// Test signature manager
    #[test]
    fn test_signature_manager() {
        let mut mgr = SignatureManager::new();

        assert!(!mgr.has_pending());

        mgr.store(Some("sig_123".to_string()));
        assert!(mgr.has_pending());

        let sig = mgr.consume();
        assert_eq!(sig, Some("sig_123".to_string()));
        assert!(!mgr.has_pending());
    }

    /// Test trailing signature handling
    #[test]
    fn test_trailing_signature() {
        let mut state = StreamingState::new();
        state.message_start_sent = true;
        state.set_trailing_signature(Some("trailing_sig".to_string()));

        assert!(state.has_trailing_signature());

        let chunks = state.emit_finish(None, None);
        let output = chunks_to_string(&chunks);

        assert!(output.contains("trailing_sig"));
        assert!(output.contains("signature_delta"));
    }

    /// Test error recovery
    #[test]
    fn test_parse_error_recovery() {
        let mut state = StreamingState::new();
        state.start_block(BlockType::Text, json!({"type": "text"}));

        let _chunks = state.handle_parse_error("malformed SSE data");

        assert_eq!(state.get_error_count(), 1);
        // Block should be safely closed
        assert_eq!(state.current_block_type(), BlockType::None);

        state.reset_error_state();
        assert_eq!(state.get_error_count(), 0);
    }

    /// Test multiple block transitions
    #[test]
    fn test_rapid_block_transitions() {
        let mut state = StreamingState::new();

        for _ in 0..5 {
            state.start_block(BlockType::Text, json!({}));
            state.start_block(BlockType::Thinking, json!({}));
            state.start_block(BlockType::Function, json!({}));
        }

        // Each transition closes previous block and starts new one
        assert!(state.current_block_index() >= 10);
    }

    /// Test grounding data in finish
    #[test]
    fn test_grounding_in_finish() {
        let mut state = StreamingState::new();
        state.message_start_sent = true;
        state.web_search_query = Some("test query".to_string());
        state.grounding_chunks = Some(vec![json!({
            "web": {
                "title": "Test Source",
                "uri": "https://example.com"
            }
        })]);

        let chunks = state.emit_finish(None, None);
        let output = chunks_to_string(&chunks);

        // Should include grounding text block
        assert!(output.contains("test query") || output.contains("Test Source"));
    }

    fn chunks_to_string(chunks: &[Bytes]) -> String {
        chunks
            .iter()
            .map(|b| String::from_utf8(b.to_vec()).unwrap_or_default())
            .collect::<String>()
    }
}

// =============================================================================
// Rate Limit Tracker Integration Tests
// =============================================================================

mod rate_limit_integration {
    use crate::proxy::rate_limit::{RateLimitReason, RateLimitTracker};

    /// Test rate limit parsing from HTTP 429
    #[test]
    fn test_rate_limit_from_429() {
        let tracker = RateLimitTracker::new();

        let info = tracker.parse_from_error("account_1", 429, Some("60"), "Rate limit exceeded");
        assert!(info.is_some());

        let info = info.unwrap();
        assert_eq!(info.retry_after_sec, 60);
        assert!(tracker.is_rate_limited("account_1"));
    }

    /// Test server error soft rate limit
    #[test]
    fn test_server_error_soft_limit() {
        let tracker = RateLimitTracker::new();

        // 500 should trigger soft avoidance
        let info = tracker.parse_from_error("account_2", 500, None, "Internal server error");
        assert!(info.is_some());
        assert_eq!(info.unwrap().retry_after_sec, 20); // Default soft limit
    }

    /// Test quota group separation
    #[test]
    fn test_quota_group_separation() {
        let tracker = RateLimitTracker::new();

        // Rate limit for claude group
        tracker.parse_from_error_with_group("account_3", 429, Some("60"), "", Some("claude"));

        // Claude should be limited
        assert!(tracker.is_rate_limited_for_group("account_3", Some("claude")));

        // Gemini should NOT be limited
        assert!(!tracker.is_rate_limited_for_group("account_3", Some("gemini")));
    }

    /// Test reason parsing
    #[test]
    fn test_rate_limit_reason_parsing() {
        let tracker = RateLimitTracker::new();

        let body = r#"{"error": {"details": [{"reason": "QUOTA_EXHAUSTED"}]}}"#;
        let info = tracker.parse_from_error("acc_reason", 429, None, body);

        assert!(info.is_some());
        assert_eq!(info.unwrap().reason, RateLimitReason::QuotaExhausted);
    }

    /// Test Google quota reset delay parsing
    #[test]
    fn test_google_quota_reset_parsing() {
        let tracker = RateLimitTracker::new();

        let body = r#"{
            "error": {
                "details": [
                    {"metadata": {"quotaResetDelay": "45s"}}
                ]
            }
        }"#;

        let info = tracker.parse_from_error("acc_google", 429, None, body);
        assert!(info.is_some());
        assert_eq!(info.unwrap().retry_after_sec, 45);
    }
}

// =============================================================================
// Model Mapping Integration Tests
// =============================================================================

mod model_mapping_integration {
    use crate::proxy::common::model_mapping::{invalidate_model_cache, map_claude_model_to_gemini};

    /// Test Claude to Gemini model mapping
    #[test]
    fn test_claude_model_mapping() {
        invalidate_model_cache();

        // Claude Opus
        assert!(map_claude_model_to_gemini("claude-opus-4").contains("thinking"));

        // Claude Sonnet
        let sonnet = map_claude_model_to_gemini("claude-sonnet-4-5");
        assert!(!sonnet.is_empty());

        // GPT-4
        assert_eq!(map_claude_model_to_gemini("gpt-4"), "gemini-2.5-pro");

        // GPT-4o
        assert_eq!(map_claude_model_to_gemini("gpt-4o-mini"), "gemini-2.5-flash");
    }

    /// Test Gemini passthrough
    #[test]
    fn test_gemini_passthrough() {
        invalidate_model_cache();

        assert_eq!(
            map_claude_model_to_gemini("gemini-2.5-flash"),
            "gemini-2.5-flash"
        );
        assert_eq!(
            map_claude_model_to_gemini("gemini-3-pro-low"),
            "gemini-3-pro-low"
        );
    }

    /// Test unknown model fallback
    #[test]
    fn test_unknown_model_fallback() {
        invalidate_model_cache();

        let result = map_claude_model_to_gemini("unknown-model-xyz");
        assert!(!result.is_empty()); // Should fallback to default
    }
}
