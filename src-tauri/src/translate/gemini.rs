//! Google Gemini 原生协议：POST `{base_url}/v1beta/models/{model}:generateContent`

use super::limiter;
use super::llm::{self, LlmChat};
use super::{BatchContext, Terms, TranslateError, TranslateItem, TranslatedItem, Translator, build_client, parse_err, require_key, send_json};
use crate::config::ProviderConfig;
use async_trait::async_trait;
use serde_json::{Value, json};

pub const DEFAULT_BASE: &str = "https://generativelanguage.googleapis.com";

pub struct Gemini {
    engine_id: String,
    concurrency: usize,
    client: reqwest::Client,
    url: String,
    api_key: String,
}

impl Gemini {
    #[allow(dead_code)]
    pub fn new(cfg: &ProviderConfig) -> Result<Self, TranslateError> {
        Self::new_with_id("gemini", cfg)
    }

    pub fn new_with_id(id: &str, cfg: &ProviderConfig) -> Result<Self, TranslateError> {
        let api_key = require_key(cfg, "Gemini API key")?;
        let model = cfg.model.trim().trim_start_matches("models/");
        if model.is_empty() {
            return Err(TranslateError::Config(t!("err.engine.no_model")));
        }
        let root = api_root(&cfg.base_url);
        Ok(Self {
            engine_id: id.to_string(),
            concurrency: cfg.effective_concurrency("gemini"),
            client: build_client(cfg)?,
            url: format!("{root}/models/{model}:generateContent"),
            api_key,
        })
    }
}

fn api_root(base: &str) -> String {
    let b = base.trim().trim_end_matches('/');
    let b = if b.is_empty() { DEFAULT_BASE } else { b };
    if b.ends_with("/v1beta") || b.ends_with("/v1") {
        b.to_string()
    } else {
        format!("{b}/v1beta")
    }
}

fn extract_content(v: &Value) -> Result<String, TranslateError> {
    let parts = v
        .pointer("/candidates/0/content/parts")
        .and_then(Value::as_array)
        .ok_or_else(|| parse_err(v))?;
    let mut text = String::new();
    let mut has_thought = false;
    for part in parts {
        if part.get("thought").and_then(Value::as_bool) == Some(true) {
            if part.get("text").and_then(Value::as_str).is_some_and(|s| !s.trim().is_empty()) {
                has_thought = true;
            }
            continue;
        }
        if let Some(s) = part.get("text").and_then(Value::as_str) {
            text.push_str(s);
        }
    }
    let clean = llm::strip_think(&text);
    if clean.is_empty() {
        if has_thought {
            return Err(TranslateError::Api(t!("err.engine.llm_only_reasoning")));
        }
        return Err(parse_err(v));
    }
    Ok(clean)
}

#[async_trait]
impl LlmChat for Gemini {
    async fn chat(&self, system: &str, user: &str, _max_tokens: u32) -> Result<String, TranslateError> {
        let v = limiter::run_limited(&self.engine_id, self.concurrency, || {
            let req = self
                .client
                .post(&self.url)
                .header("x-goog-api-key", &self.api_key)
                .json(&json!({
                    "systemInstruction": { "parts": [{ "text": system }] },
                    "contents": [{ "role": "user", "parts": [{ "text": user }] }],
                    "generationConfig": { "temperature": 0.2 },
                }));
            async move { send_json(req, "/error/message").await }
        })
        .await
        .map_err(|e| llm::auth_hint(e, &self.url, &self.api_key))?;
        extract_content(&v)
    }
}

pub async fn list_models(cfg: &ProviderConfig) -> Result<Vec<String>, TranslateError> {
    let key = require_key(cfg, "Gemini API key")?;
    let url = format!("{}/models?pageSize=1000", api_root(&cfg.base_url));
    let req = build_client(cfg)?.get(&url).header("x-goog-api-key", &key);
    let v = send_json(req, "/error/message").await.map_err(|e| llm::auth_hint(e, &url, &key))?;
    let list = v.pointer("/models").and_then(Value::as_array).ok_or_else(|| parse_err(&v))?;
    let mut ids: Vec<String> = list
        .iter()
        .filter(|m| {
            m.get("supportedGenerationMethods")
                .and_then(Value::as_array)
                .is_none_or(|methods| methods.iter().any(|x| x.as_str() == Some("generateContent")))
        })
        .filter_map(|m| m.get("name").and_then(Value::as_str))
        .map(|s| s.trim_start_matches("models/").to_string())
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[async_trait]
impl Translator for Gemini {
    async fn translate(&self, text: &str, target_lang: &str) -> Result<String, TranslateError> {
        llm::translate_one(self, text, target_lang).await
    }

    async fn translate_batch(&self, texts: &[String], target_lang: &str) -> Result<Vec<String>, TranslateError> {
        llm::translate_batch(self, texts, target_lang).await
    }

    async fn translate_items(
        &self,
        items: &[TranslateItem],
        ctx: &BatchContext,
        terms: &Terms,
        target_lang: &str,
    ) -> Result<Vec<TranslatedItem>, TranslateError> {
        llm::translate_items(self, items, ctx, terms, target_lang).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 根地址拼接() {
        assert_eq!(api_root(""), "https://generativelanguage.googleapis.com/v1beta");
        assert_eq!(api_root("https://generativelanguage.googleapis.com/"), "https://generativelanguage.googleapis.com/v1beta");
        assert_eq!(api_root("https://proxy.example.com/v1beta"), "https://proxy.example.com/v1beta");
        assert_eq!(api_root("https://proxy.example.com/v1"), "https://proxy.example.com/v1");
    }

    #[test]
    fn 解析候选文本与思考部分() {
        let normal = json!({ "candidates": [{ "content": { "parts": [{ "text": "你好" }] } }] });
        assert_eq!(extract_content(&normal).unwrap(), "你好");

        let with_thought = json!({
            "candidates": [{
                "content": {
                    "parts": [
                        { "thought": true, "text": "先思考" },
                        { "text": " 译文 " }
                    ]
                }
            }]
        });
        assert_eq!(extract_content(&with_thought).unwrap(), "译文");

        let only_thought = json!({ "candidates": [{ "content": { "parts": [{ "thought": true, "text": "思考" }] } }] });
        assert!(matches!(extract_content(&only_thought), Err(TranslateError::Api(_))));
    }
}
