use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::AiConfig;
use crate::history::{AiItem, AiItemKind};

pub const AI_SYSTEM_PROMPT: &str = "You generate shell command candidates for Aish. Return final JSON only. Do not include reasoning, markdown, or prose. The JSON must match {\"items\":[{\"kind\":\"command\",\"text\":\"...\"}]} or template items with kind=\"template\", name, and text. Answer only the current user request; do not repeat unrelated examples or previous commands. For concrete values, emit a concrete command. For generic words such as something, message, file, path, pattern, name, or value, use explicit brace placeholders in the command text, for example echo {message}, instead of treating those words as literal arguments. Use template items for reusable command shapes. Do not include secrets or unrelated commands.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AiRequest {
    pub url: String,
    pub api_key: String,
    pub body: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct AiResponse {
    items: Vec<AiResponseItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct AiResponseItem {
    kind: AiResponseItemKind,
    text: String,
    name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AiResponseItemKind {
    Command,
    Template,
}

pub fn normalize_chat_completions_url(base_url: &str) -> Result<String> {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        bail!("AI base URL is not configured");
    }
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        bail!("AI base URL must start with http:// or https://");
    }
    if trimmed.ends_with("/chat/completions") {
        Ok(trimmed.to_string())
    } else {
        Ok(format!("{trimmed}/chat/completions"))
    }
}

pub fn read_api_key_from_env(env_key: &str) -> Result<String> {
    let env_key = env_key.trim();
    if env_key.is_empty() {
        bail!("AI API key environment variable is not configured");
    }
    let value = std::env::var(env_key)
        .map_err(|_| anyhow!("AI API key environment variable is not set: {env_key}"))?;
    if value.trim().is_empty() {
        bail!("AI API key environment variable is empty: {env_key}");
    }
    Ok(value)
}

pub fn build_ai_request(config: &AiConfig, prompt: &str) -> Result<AiRequest> {
    if config.model.trim().is_empty() {
        bail!("AI model is not configured");
    }
    let url = normalize_chat_completions_url(&config.base_url)?;
    let api_key = read_api_key_from_env(&config.env_key)?;
    let body = build_chat_completions_body(&config.model, prompt);
    Ok(AiRequest { url, api_key, body })
}

pub fn request_ai_items(config: &AiConfig, prompt: &str) -> Result<Vec<AiItem>> {
    let request = build_ai_request(config, prompt)?;
    let raw = send_chat_completions_request(&request)?;
    let content = extract_chat_message_content(&raw)?;
    parse_ai_items(&content)
}

pub fn send_chat_completions_request(request: &AiRequest) -> Result<String> {
    let response = reqwest::blocking::Client::new()
        .post(&request.url)
        .bearer_auth(&request.api_key)
        .json(&request.body)
        .send()
        .map_err(|error| anyhow!("AI request failed: {error}"))?;
    let status = response.status();
    let body = response
        .text()
        .map_err(|error| anyhow!("AI response body could not be read: {error}"))?;
    if !status.is_success() {
        bail!("AI request failed with status {status}: {body}");
    }
    Ok(body)
}

pub fn extract_chat_message_content(raw: &str) -> Result<String> {
    let parsed: Value = serde_json::from_str(raw)
        .map_err(|error| anyhow!("AI provider response was not valid JSON: {error}"))?;
    parsed
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("AI provider response did not contain choices[0].message.content"))
}

pub fn build_chat_completions_body(model: &str, prompt: &str) -> Value {
    json!({
        "model": model,
        "messages": [
            { "role": "system", "content": AI_SYSTEM_PROMPT },
            { "role": "user", "content": prompt }
        ],
        "response_format": { "type": "json_object" }
    })
}

pub fn parse_ai_items(raw: &str) -> Result<Vec<AiItem>> {
    let parsed: AiResponse = serde_json::from_str(raw).map_err(|error| {
        anyhow!("AI response was not valid JSON matching the expected schema: {error}")
    })?;
    if parsed.items.is_empty() {
        bail!("AI response did not contain any items");
    }

    parsed
        .items
        .into_iter()
        .enumerate()
        .map(|(index, item)| validate_ai_item(index, item))
        .collect()
}

fn validate_ai_item(index: usize, item: AiResponseItem) -> Result<AiItem> {
    let text = item.text.trim().to_string();
    if text.is_empty() {
        bail!("AI item {index} has empty text");
    }
    match item.kind {
        AiResponseItemKind::Command => Ok(AiItem {
            kind: AiItemKind::Command,
            text,
            name: None,
        }),
        AiResponseItemKind::Template => {
            let Some(name) = item.name.map(|name| name.trim().to_string()) else {
                bail!("AI template item {index} is missing name");
            };
            if name.is_empty() {
                bail!("AI template item {index} has empty name");
            }
            Ok(AiItem {
                kind: AiItemKind::Template,
                text,
                name: Some(name),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_chat_completions_url_appends_endpoint() {
        assert_eq!(
            normalize_chat_completions_url("https://example.invalid/v1/").unwrap(),
            "https://example.invalid/v1/chat/completions"
        );
        assert_eq!(
            normalize_chat_completions_url("https://example.invalid/v1/chat/completions").unwrap(),
            "https://example.invalid/v1/chat/completions"
        );
    }

    #[test]
    fn normalize_chat_completions_url_rejects_missing_scheme_or_empty() {
        assert!(
            normalize_chat_completions_url("")
                .unwrap_err()
                .to_string()
                .contains("not configured")
        );
        assert!(
            normalize_chat_completions_url("example.invalid/v1")
                .unwrap_err()
                .to_string()
                .contains("must start")
        );
    }

    #[test]
    fn build_chat_completions_body_uses_strict_json_prompt() {
        let body = build_chat_completions_body("gpt-test", "list files");

        assert_eq!(body["model"], "gpt-test");
        assert_eq!(body["response_format"]["type"], "json_object");
        assert_eq!(body["messages"][0]["role"], "system");
        assert!(
            body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("final JSON only")
        );
        assert!(
            body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("echo {message}")
        );
        assert!(
            body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("do not repeat unrelated examples")
        );
        assert_eq!(body["messages"][1]["content"], "list files");
    }

    #[test]
    fn extract_chat_message_content_reads_first_choice_message() {
        let content = extract_chat_message_content(
            r#"{"choices":[{"message":{"content":"{\"items\":[{\"kind\":\"command\",\"text\":\"pwd\"}]}"}}]}"#,
        )
        .unwrap();

        assert_eq!(content, r#"{"items":[{"kind":"command","text":"pwd"}]}"#);
    }

    #[test]
    fn extract_chat_message_content_rejects_missing_content() {
        assert!(
            extract_chat_message_content(r#"{"choices":[{"message":{}}]}"#)
                .unwrap_err()
                .to_string()
                .contains("choices[0].message.content")
        );
    }

    #[test]
    fn parse_ai_items_accepts_command_and_template_items() {
        let items = parse_ai_items(
            r#"{"items":[{"kind":"command","text":"git status"},{"kind":"template","name":"deploy","text":"rsync {from} {to}"}]}"#,
        )
        .unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, AiItemKind::Command);
        assert_eq!(items[0].text, "git status");
        assert_eq!(items[0].name, None);
        assert_eq!(items[1].kind, AiItemKind::Template);
        assert_eq!(items[1].name.as_deref(), Some("deploy"));
    }

    #[test]
    fn parse_ai_items_rejects_empty_or_invalid_items() {
        assert!(
            parse_ai_items(r#"{"items":[]}"#)
                .unwrap_err()
                .to_string()
                .contains("any items")
        );
        assert!(
            parse_ai_items(r#"{"items":[{"kind":"command","text":""}]}"#)
                .unwrap_err()
                .to_string()
                .contains("empty text")
        );
        assert!(
            parse_ai_items(r#"{"items":[{"kind":"template","text":"body"}]}"#)
                .unwrap_err()
                .to_string()
                .contains("missing name")
        );
    }
}
