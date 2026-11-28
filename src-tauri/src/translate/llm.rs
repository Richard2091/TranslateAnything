//! 大模型翻译公共逻辑：提示词、批量分片、思考标签清理、结构化解析、鉴权提示

use super::TranslateError;
use async_trait::async_trait;
use futures_util::future::join_all;
use serde_json::Value;

/// 一次批量请求最多带多少条原文 / 多少字，超出的拆成多个请求并发发出
pub const BATCH_ITEMS: usize = 40;
pub const BATCH_CHARS: usize = 6000;
/// 退回逐条翻译时的并发数
pub const SINGLE_CONCURRENCY: usize = 4;

/// 单个模型对话轮次的默认最大输出 token（兼容 Anthropic 等必填该字段的协议）
pub const DEFAULT_MAX_TOKENS: u32 = 4096;

#[async_trait]
pub trait LlmChat: Send + Sync {
    async fn chat(&self, system: &str, user: &str, max_tokens: u32) -> Result<String, TranslateError>;
}

pub fn system_prompt(target_lang: &str) -> String {
    format!(
        "你是专业的翻译引擎。请把用户发来的文本翻译成目标语言（语言代码：{target_lang}）。\
         文本来自屏幕截图的 OCR 结果，可能存在被硬换行打断的句子和个别识别错误：\
         请按语义合并被打断的句子、保留原有的段落和列表结构、修正明显的识别错误。\
         如果文本本身已经是目标语言，原样输出。\
         只输出译文，不要任何解释、前后缀或引号。"
    )
}

pub fn batch_prompt(target_lang: &str) -> String {
    format!(
        "你是专业的翻译引擎。用户发来一个 JSON 字符串数组，每一项是屏幕上的一段文字（来自 OCR，可能有个别识别错误）。\
         请把每一项分别翻译成目标语言（语言代码：{target_lang}），修正明显的识别错误；已经是目标语言的项原样保留。\
         只输出一个 JSON 字符串数组：项数与输入完全相同、顺序一一对应，不要合并或拆分条目，不要任何解释或代码块标记。"
    )
}

/// 去掉思考模型输出里的 `<think>…</think>`（部分接口把思考过程混在正文里返回）
pub fn strip_think(s: &str) -> String {
    let mut s = s.trim();
    if let Some(rest) = s.strip_prefix("<think>") {
        s = rest.split_once("</think>").map_or("", |(_, after)| after).trim();
    }
    s.to_string()
}

/// 解析批量回复：JSON 字符串数组（容忍 ```json 代码块、前后多余文字、`{"translations":[…]}` 包一层）
pub fn parse_batch(reply: &str, n: usize) -> Option<Vec<String>> {
    let start = reply.find(['[', '{'])?;
    let end = reply.rfind([']', '}'])?;
    let v: Value = serde_json::from_str(reply.get(start..=end)?).ok()?;
    let arr = match &v {
        Value::Array(a) => a,
        Value::Object(o) => o.values().find_map(Value::as_array)?,
        _ => return None,
    };
    let out: Vec<String> = arr.iter().map(|x| x.as_str().map(|s| s.trim().to_string())).collect::<Option<_>>()?;
    (out.len() == n).then_some(out)
}

/// 鉴权失败（401 / 403）时补一句排查提示：多半是密钥和接口地址不是同一家服务商
pub fn auth_hint(e: TranslateError, url: &str, key: &str) -> TranslateError {
    match e {
        TranslateError::Api(m) if m.starts_with("HTTP 401") || m.starts_with("HTTP 403") => {
            let host = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
            let mut m = format!("{m}\n{}", t!("err.engine.llm_auth_hint", host = host));
            if host.ends_with("deepseek.com") && !key.starts_with("sk-") {
                m.push('\n');
                m.push_str(&t!("err.engine.deepseek_key_hint"));
            }
            TranslateError::Api(m)
        }
        e => e,
    }
}

pub async fn translate_one<C: LlmChat>(client: &C, text: &str, target_lang: &str) -> Result<String, TranslateError> {
    client.chat(&system_prompt(target_lang), text, DEFAULT_MAX_TOKENS).await
}

async fn translate_each<C: LlmChat>(client: &C, texts: &[String], target_lang: &str) -> Result<Vec<String>, TranslateError> {
    let mut out = Vec::with_capacity(texts.len());
    for chunk in texts.chunks(SINGLE_CONCURRENCY) {
        for r in join_all(chunk.iter().map(|t| translate_one(client, t, target_lang))).await {
            out.push(r?);
        }
    }
    Ok(out)
}

pub async fn translate_batch<C: LlmChat>(client: &C, texts: &[String], target_lang: &str) -> Result<Vec<String>, TranslateError> {
    if texts.len() <= 1 {
        return translate_each(client, texts, target_lang).await;
    }
    let mut groups: Vec<&[String]> = Vec::new();
    let (mut start, mut chars) = (0, 0);
    for (i, t) in texts.iter().enumerate() {
        let len = t.chars().count();
        if i > start && (i - start >= BATCH_ITEMS || chars + len > BATCH_CHARS) {
            groups.push(&texts[start..i]);
            (start, chars) = (i, 0);
        }
        chars += len;
    }
    groups.push(&texts[start..]);
    let sys = batch_prompt(target_lang);
    let results = join_all(groups.iter().map(|g| {
        let input = serde_json::to_string(g).unwrap_or_default();
        let sys = &sys;
        async move {
            let reply = client.chat(sys, &input, DEFAULT_MAX_TOKENS).await?;
            Ok::<Option<Vec<String>>, TranslateError>(parse_batch(&reply, g.len()))
        }
    }))
    .await;
    let mut out = Vec::with_capacity(texts.len());
    for (g, r) in groups.iter().zip(results) {
        match r? {
            Some(v) => out.extend(v),
            None => out.extend(translate_each(client, g, target_lang).await?),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 解析批量回复() {
        assert_eq!(parse_batch(r#"["你好","世界"]"#, 2), Some(vec!["你好".into(), "世界".into()]));
        assert_eq!(parse_batch("```json\n[\"a\", \"b\"]\n```", 2), Some(vec!["a".into(), "b".into()]));
        assert_eq!(parse_batch(r#"{"translations":["a"]}"#, 1), Some(vec!["a".into()]));
        assert_eq!(parse_batch(r#"["a"]"#, 2), None, "条数不符");
        assert_eq!(parse_batch("你好", 1), None);
    }

    #[test]
    fn 去掉思考过程() {
        assert_eq!(strip_think("<think>嗯……</think>\n你好"), "你好");
        assert_eq!(strip_think("  你好 "), "你好");
    }
}
