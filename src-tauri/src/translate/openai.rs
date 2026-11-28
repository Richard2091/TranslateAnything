//! OpenAI 兼容协议：POST `{base_url}/chat/completions`

use super::llm::{self, LlmChat};
use super::{TranslateError, Translator, build_client, parse_err, send_json};
use crate::config::ProviderConfig;
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct OpenAICompatible {
    client: reqwest::Client,
    url: String,
    api_key: String,
    model: String,
}

impl OpenAICompatible {
    pub fn new(cfg: &ProviderConfig) -> Result<Self, TranslateError> {
        let base = cfg.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            return Err(TranslateError::Config(t!("err.engine.no_base_url")));
        }
        if cfg.model.trim().is_empty() {
            return Err(TranslateError::Config(t!("err.engine.no_model")));
        }
        Ok(Self {
            client: build_client(cfg)?,
            url: chat_url(base),
            api_key: cfg.api_key.trim().to_string(),
            model: cfg.model.trim().to_string(),
        })
    }
}

#[async_trait]
impl LlmChat for OpenAICompatible {
    async fn chat(&self, system: &str, user: &str, _max_tokens: u32) -> Result<String, TranslateError> {
        let mut req = self.client.post(&self.url).json(&json!({
            "model": self.model,
            "temperature": 0.2,
            "stream": false,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        }));
        if !self.api_key.is_empty() {
            req = req.bearer_auth(&self.api_key);
        }
        let v = send_json(req, "/error/message").await.map_err(|e| llm::auth_hint(e, &self.url, &self.api_key))?;
        let msg = v.pointer("/choices/0/message").ok_or_else(|| parse_err(&v))?;
        let content = msg.get("content").and_then(Value::as_str).map(llm::strip_think).unwrap_or_default();
        if content.is_empty() {
            // 思考模型把预算全花在思考上、正文为空（如 deepseek-reasoner 被截断），给出明确提示
            if msg.get("reasoning_content").and_then(Value::as_str).is_some_and(|s| !s.is_empty()) {
                return Err(TranslateError::Api(t!("err.engine.llm_only_reasoning")));
            }
            return Err(parse_err(&v));
        }
        Ok(content)
    }
}

/// 对话接口地址：已经写到 `/chat/completions` 的原样使用
fn chat_url(base: &str) -> String {
    if base.ends_with("/chat/completions") { base.to_string() } else { format!("{base}/chat/completions") }
}

/// 列出接口提供的模型。各家兼容接口基本都实现了 `GET /models`（返回 `data[].id`）；
/// Ollama 原生接口返回 `models[].name`，一并兼容
pub async fn list_models(cfg: &ProviderConfig) -> Result<Vec<String>, TranslateError> {
    let base = cfg.base_url.trim().trim_end_matches('/');
    let base = base.strip_suffix("/chat/completions").unwrap_or(base);
    if base.is_empty() {
        return Err(TranslateError::Config(t!("err.engine.no_base_url")));
    }
    let mut req = build_client(cfg)?.get(format!("{base}/models"));
    let key = cfg.api_key.trim();
    if !key.is_empty() {
        req = req.bearer_auth(key);
        // 兼容用 OpenAI 类型条目但指向 Anthropic 官方地址的场景
        if base.contains("anthropic.com") {
            req = req.header("x-api-key", key).header("anthropic-version", "2023-06-01");
        }
    }
    let v = send_json(req, "/error/message").await.map_err(|e| llm::auth_hint(e, base, key))?;
    let list = v
        .pointer("/data")
        .or_else(|| v.pointer("/models"))
        .and_then(Value::as_array)
        .ok_or_else(|| parse_err(&v))?;
    // Gemini 的 id 带 "models/" 前缀，对话接口不需要
    let mut ids: Vec<String> = list
        .iter()
        .filter_map(|m| m.get("id").or_else(|| m.get("name")).and_then(Value::as_str))
        .map(|s| s.trim_start_matches("models/").to_string())
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[async_trait]
impl Translator for OpenAICompatible {
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
    fn 对话地址() {
        assert_eq!(chat_url("https://api.deepseek.com"), "https://api.deepseek.com/chat/completions");
        assert_eq!(chat_url("https://x/v1/chat/completions"), "https://x/v1/chat/completions");
    }
}
