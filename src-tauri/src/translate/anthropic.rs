//! Anthropic Messages 原生协议：POST `{base_url}/v1/messages`

use super::llm::{self, LlmChat};
use super::{TranslateError, Translator, build_client, parse_err, require_key, send_json};
use crate::config::ProviderConfig;
use async_trait::async_trait;
use serde_json::{Value, json};

pub const DEFAULT_BASE: &str = "https://api.anthropic.com";
const API_VERSION: &str = "2023-06-01";

pub struct Anthropic {
    client: reqwest::Client,
    url: String,
    api_key: String,
    model: String,
}

impl Anthropic {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, TranslateError> {
        let api_key = require_key(cfg, "Anthropic API key")?;
        if cfg.model.trim().is_empty() {
            return Err(TranslateError::Config(t!("err.engine.no_model")));
        }
        Ok(Self {
            client: build_client(cfg)?,
            url: messages_url(&cfg.base_url),
            api_key,
            model: cfg.model.trim().to_string(),
        })
    }
}

fn normalized_base(base: &str) -> &str {
    let b = base.trim().trim_end_matches('/');
    if b.is_empty() { DEFAULT_BASE } else { b }
}

fn messages_url(base: &str) -> String {
    let b = normalized_base(base);
    if b.ends_with("/messages") {
        b.to_string()
    } else if b.ends_with("/v1") {
        format!("{b}/messages")
    } else {
        format!("{b}/v1/messages")
    }
}

fn models_url(base: &str) -> String {
    let b = normalized_base(base);
    let b = b.strip_suffix("/messages").unwrap_or(b);
    if b.ends_with("/v1") {
        format!("{b}/models")
    } else {
        format!("{b}/v1/models")
    }
}

fn extract_content(v: &Value) -> Result<String, TranslateError> {
    let blocks = v.get("content").and_then(Value::as_array).ok_or_else(|| parse_err(v))?;
    let mut text = String::new();
    let mut has_thinking = false;
    for item in blocks {
        match item.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(s) = item.get("text").and_then(Value::as_str) {
                    text.push_str(s);
                }
            }
            Some("thinking") => {
                if item.get("thinking").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty()) {
                    has_thinking = true;
                }
            }
            _ => {}
        }
    }
    let clean = llm::strip_think(&text);
    if clean.is_empty() {
        if has_thinking {
            return Err(TranslateError::Api(t!("err.engine.llm_only_reasoning")));
        }
        return Err(parse_err(v));
    }
    Ok(clean)
}

#[async_trait]
impl LlmChat for Anthropic {
    async fn chat(&self, system: &str, user: &str, max_tokens: u32) -> Result<String, TranslateError> {
        let req = self
            .client
            .post(&self.url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&json!({
                "model": self.model,
                "max_tokens": max_tokens,
                "temperature": 0.2,
                "system": system,
                "messages": [
                    { "role": "user", "content": user },
                ],
            }));
        let v = send_json(req, "/error/message").await.map_err(|e| llm::auth_hint(e, &self.url, &self.api_key))?;
        extract_content(&v)
    }
}

pub async fn list_models(cfg: &ProviderConfig) -> Result<Vec<String>, TranslateError> {
    let key = require_key(cfg, "Anthropic API key")?;
    let url = models_url(&cfg.base_url);
    let req = build_client(cfg)?
        .get(&url)
        .header("x-api-key", &key)
        .header("anthropic-version", API_VERSION);
    let v = send_json(req, "/error/message").await.map_err(|e| llm::auth_hint(e, &url, &key))?;
    let list = v.pointer("/data").and_then(Value::as_array).ok_or_else(|| parse_err(&v))?;
    let mut ids: Vec<String> = list
        .iter()
        .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_string))
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[async_trait]
impl Translator for Anthropic {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError> {
        llm::translate_one(self, text, target_lang).await
    }

    async fn translate_batch(&self, texts: &[String], target_lang: &str) -> Result<Vec<String>, TranslateError> {
        llm::translate_batch(self, texts, target_lang).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 地址拼接() {
        assert_eq!(messages_url(""), "https://api.anthropic.com/v1/messages");
        assert_eq!(messages_url("https://api.anthropic.com/"), "https://api.anthropic.com/v1/messages");
        assert_eq!(messages_url("https://proxy.example.com/v1"), "https://proxy.example.com/v1/messages");
        assert_eq!(messages_url("https://proxy.example.com/v1/messages"), "https://proxy.example.com/v1/messages");
        assert_eq!(models_url(""), "https://api.anthropic.com/v1/models");
        assert_eq!(models_url("https://proxy.example.com/v1/messages"), "https://proxy.example.com/v1/models");
    }

    #[test]
    fn 解析回复与思考块() {
        let normal = json!({ "content": [{ "type": "text", "text": "你好" }] });
        assert_eq!(extract_content(&normal).unwrap(), "你好");

        let mixed = json!({
            "content": [
                { "type": "thinking", "thinking": "思考中" },
                { "type": "text", "text": " 世界 " }
            ]
        });
        assert_eq!(extract_content(&mixed).unwrap(), "世界");

        let only_think = json!({ "content": [{ "type": "thinking", "thinking": "只思考" }] });
        assert!(matches!(extract_content(&only_think), Err(TranslateError::Api(_))));
    }
}
