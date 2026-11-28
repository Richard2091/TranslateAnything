//! 版面分析：把 OCR 的行/单词归并成"段落块"，并为每块取样背景色与文字色，
//! 供前端按原位置覆盖渲染译文（沉浸式效果）。

use crate::ocr::{Line, OcrOutput, Word};
use image::RgbaImage;
use serde::Serialize;

/// 一行内单词间距超过 行高 × 该值 时拆成两段（多栏、表格单元格）
const SPLIT_GAP: f32 = 1.6;
/// 相邻两行的行高比在此范围内才可能属于同一段
const HEIGHT_RATIO: (f32, f32) = (0.7, 1.45);
/// 相邻两行的垂直间距不超过 行高 × 该值 才属于同一段
const MAX_LINE_GAP: f32 = 0.9;
/// 相邻两行文字色差（RGB 各通道差之和）超过该值视为不同组件（如深色标题 + 灰色说明）
const MAX_COLOR_DIFF: u32 = 80;
/// 相邻两行估算字号之比在此范围内才可能属于同一段（大小标题、标题与正文分开）
const FONT_RATIO: (f32, f32) = (0.85, 1.18);

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn right(&self) -> f32 {
        self.x + self.w
    }
    fn bottom(&self) -> f32 {
        self.y + self.h
    }
    fn union(&self, o: &Rect) -> Rect {
        let x = self.x.min(o.x);
        let y = self.y.min(o.y);
        Rect {
            x,
            y,
            w: self.right().max(o.right()) - x,
            h: self.bottom().max(o.bottom()) - y,
        }
    }
}

/// 一行中的一个片段（按大间距拆开后的连续单词）
#[derive(Debug, Clone)]
struct Segment {
    text: String,
    rect: Rect,
    /// 各单词的（文字，框高），用于估算字号
    words: Vec<(String, f32)>,
    /// 该行的文字色与估算字号（归并时用来区分组件）
    fg: Rgb,
    em: f32,
    bg: Rgb,
    marker: Option<LineMarker>,
    orphan_dot_body_x: Option<f32>,
    heading: bool,
}

#[derive(Debug, Clone)]
struct LineMarker {
    text: String,
    body_x: f32,
    x: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ListMarker {
    pub text: String,
    /// 从块左边缘到列表正文起点的物理像素距离
    pub indent: f32,
    /// 从块左边缘到标记起点的物理像素距离
    pub offset: f32,
}

/// 段落块：前端在 `rect` 处盖上 `bg` 色块并以 `fg` 色显示译文
#[derive(Debug, Clone, Serialize)]
pub struct Block {
    /// 原文（已合并成一段）
    pub source: String,
    /// 列表标记单独渲染，不进入翻译文本或缓存键
    #[serde(skip_serializing_if = "Option::is_none")]
    pub marker: Option<ListMarker>,
    /// 物理像素，相对截图左上角
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// 行数与平均行高（OCR 框高）
    pub lines: u32,
    pub line_h: f32,
    /// 估算出的原文字号（em，物理像素）：译文用同样的字号显示
    pub font: f32,
    /// 原文行距（相邻两行基线间距，物理像素）；单行块为 0，前端让文字在块内垂直居中
    pub pitch: f32,
    /// 多行块首行的上留白（物理像素），让译文首行与原文首行对齐
    pub pad_top: f32,
    /// 字重：400 常规 / 600 半粗 / 700 粗体
    pub weight: u16,
    /// 对齐方式：`left` / `center`
    pub align: &'static str,
    /// `#rrggbb`
    pub bg: String,
    pub fg: String,
    /// 是否为标题片段（仅后端传上下文用，不序列化给前端）
    #[serde(skip)]
    pub heading: bool,
}

/// OCR 结果 → 段落块；`skip_cjk` 为真时跳过本身就是中日文的块（目标语言为中文时不必翻译）
#[cfg_attr(not(test), allow(dead_code))]
pub fn build_blocks(ocr: &OcrOutput, image: &RgbaImage, skip_cjk: bool) -> Vec<Block> {
    let mut cfg = crate::config::Config::default();
    cfg.target_lang = if skip_cjk { "zh-CN".into() } else { "xx".into() };
    build_blocks_for(ocr, image, &cfg, |_, _| ("", ""), false)
}

/// OCR 结果 → 段落块，使用统一过滤规则与按块归属的应用判断
pub fn build_blocks_for<'a>(
    ocr: &OcrOutput,
    image: &RgbaImage,
    cfg: &crate::config::Config,
    app_at: impl Fn(f32, f32) -> (&'a str, &'a str),
    record_filter: bool,
) -> Vec<Block> {
    let (iw, ih) = (image.width() as f32, image.height() as f32);
    let mut segments: Vec<Segment> = ocr
        .lines
        .iter()
        .flat_map(|l| split_line(l, ocr.cjk))
        // 贴着上下边缘的行多半被截断，识别结果不可靠
        .filter(|s| s.rect.y > 1.0 && s.rect.bottom() < ih - 1.0)
        .collect();
    for seg in &mut segments {
        let r = &seg.rect;
        let probe = Rect { x: r.x - 2.0, y: r.y - 2.0, w: r.w + 4.0, h: r.h + 4.0 };
        let (bg, fg) = sample_colors(image, &probe);
        seg.bg = bg;
        seg.fg = fg;
        seg.em = estimate_font(std::slice::from_ref(seg), r.h, ocr.cjk);
    }
    segments.sort_by(|a, b| a.rect.y.total_cmp(&b.rect.y));
    recover_first_number(&mut segments, image);
    recover_bullets(&mut segments, image);
    mark_headings(&mut segments, image, ocr.cjk);

    // 贪心归并：列表项、标题各自起块，普通段落沿用行高、间距和重叠判断。
    let mut groups: Vec<Vec<Segment>> = Vec::new();
    for seg in segments {
        if seg.marker.is_some() || seg.heading {
            groups.push(vec![seg]);
            continue;
        }
        let h = seg.rect.h;
        let target = groups.iter_mut().rev().find(|g| {
            let last = g.last().unwrap();
            let ratio = h / last.rect.h;
            let gap = seg.rect.y - last.rect.bottom();
            let overlap = seg.rect.right().min(last.rect.right()) - seg.rect.x.max(last.rect.x);
            let font_ratio = seg.em / last.em.max(1.0);
            let first = &g[0];
            !first.heading
                && first.marker.as_ref().is_none_or(|m| {
                    (seg.rect.x - m.body_x).abs() <= seg.em * 0.35
                })
                && ratio > HEIGHT_RATIO.0
                && ratio < HEIGHT_RATIO.1
                && font_ratio > FONT_RATIO.0
                && font_ratio < FONT_RATIO.1
                && dist(seg.fg, last.fg) <= MAX_COLOR_DIFF
                && gap > -0.3 * h
                && gap < MAX_LINE_GAP * h
                && overlap > 0.0
        });
        match target {
            Some(g) => g.push(seg),
            None => groups.push(vec![seg]),
        }
    }

    groups
        .into_iter()
        .filter_map(|g| {
            let rect = g.iter().skip(1).fold(g[0].rect, |r, s| r.union(&s.rect));
            let source = join_lines(g.iter().map(|s| s.text.as_str()), ocr.cjk);
            let (app_id, app_name) = app_at(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            let ctx = crate::filter::FilterContext {
                target_lang: &cfg.target_lang,
                app_id,
                app_name,
                mode: crate::filter::Mode::Overlay,
            };
            match crate::filter::classify(cfg, &source, &ctx) {
                crate::filter::Decision::Keep(reason) => {
                    if record_filter {
                        crate::filter::record_event(&source, "keep", reason, &ctx);
                    }
                    return None;
                }
                crate::filter::Decision::ForceTranslate => {
                    if record_filter {
                        crate::filter::record_event(&source, "translate", crate::filter::Reason::ForceTranslate, &ctx);
                    }
                }
                crate::filter::Decision::Translate => {}
            }
            let lines = g.len() as u32;
            let line_h = g.iter().map(|s| s.rect.h).sum::<f32>() / lines as f32;
            // 色块四周外扩，确保完整盖住原文笔画（OCR 框不一定包住升部、降部和抗锯齿边缘）
            let pad = (line_h * 0.18).clamp(2.0, 5.0);
            let x = (rect.x - pad).max(0.0);
            let y = (rect.y - pad).max(0.0);
            let cover = Rect {
                x,
                y,
                w: (rect.right() + pad).min(iw) - x,
                h: (rect.bottom() + pad).min(ih) - y,
            };
            let (bg, fg) = sample_colors(image, &cover);
            let font = estimate_font(&g, line_h, ocr.cjk);
            let pitch = if lines > 1 { line_pitch(&g) } else { 0.0 };
            // 首行行框中心对齐原文首行中心
            let first_center = g[0].rect.y + g[0].rect.h / 2.0 - cover.y;
            let pad_top = if lines > 1 { (first_center - pitch / 2.0).max(0.0) } else { 0.0 };
            let stroke = stroke_width(image, &rect, bg, fg);
            let weight = weight_from_stroke(stroke, font, luma(fg) > luma(bg));
            let marker = g[0].marker.as_ref().map(|m| ListMarker {
                text: m.text.clone(),
                indent: m.body_x - cover.x,
                offset: m.x - cover.x,
            });
            let align = if marker.is_some() || g[0].heading {
                "left"
            } else if lines > 1 {
                detect_align(&g)
            } else {
                boxed_align(image, &cover, bg, font)
            };
            Some(Block {
                source,
                marker,
                x: cover.x,
                y: cover.y,
                w: cover.w,
                h: cover.h,
                lines,
                line_h,
                font,
                pitch,
                pad_top,
                weight,
                align,
                bg: hex(bg),
                fg: hex(fg),
                heading: g[0].heading,
            })
        })
        .collect()
}

/// 按单词间的大间距把一行拆成多个片段
fn split_line(line: &Line, cjk: bool) -> Vec<Segment> {
    let line_h = line.words.iter().map(|w| w.h).fold(0.0, f32::max).max(1.0);
    let mut out = Vec::new();
    let mut cur: Vec<&Word> = Vec::new();
    for w in &line.words {
        if let Some(prev) = cur.last() {
            let gap = w.x - (prev.x + prev.w);
            let after_marker = cur.len() == 1
                && marker_prefix(&prev.text) == Some(prev.text.as_str())
                && gap <= 2.5 * line_h;
            if gap > SPLIT_GAP * line_h && !after_marker {
                out.push(make_segment(&cur, cjk));
                cur.clear();
            }
        }
        cur.push(w);
    }
    if !cur.is_empty() {
        out.push(make_segment(&cur, cjk));
    }
    out
}

fn make_segment(words: &[&Word], cjk: bool) -> Segment {
    let mut rect = Rect {
        x: words[0].x,
        y: words[0].y,
        w: words[0].w,
        h: words[0].h,
    };
    let mut text = String::new();
    let mut metrics = Vec::with_capacity(words.len());
    let prefix = marker_prefix(&words[0].text);
    let marker = prefix.and_then(|prefix| {
        let first = words[0];
        let tail = &first.text[prefix.len()..];
        let body_x = if tail.is_empty() {
            words.get(1)?.x
        } else {
            // OCR 有时把标记和第一个词塞进同一个框，没有字符级坐标。
            first.x + first.w * prefix.chars().count() as f32 / first.text.chars().count() as f32
        };
        Some(LineMarker { text: prefix.into(), body_x, x: first.x })
    });
    for (i, w) in words.iter().enumerate() {
        rect = rect.union(&Rect { x: w.x, y: w.y, w: w.w, h: w.h });
        let body = match &marker {
            Some(m) if i == 0 => &w.text[m.text.len()..],
            _ => &w.text,
        };
        if !body.is_empty() {
            metrics.push((body.to_string(), w.h));
            push_word(&mut text, body, cjk);
        }
    }
    Segment {
        text,
        rect,
        words: metrics,
        fg: [0, 0, 0],
        em: 0.0,
        bg: [0, 0, 0],
        marker,
        orphan_dot_body_x: if words[0].text == "." { words.get(1).map(|w| w.x) } else { None },
        heading: false,
    }
}

/// 识别行首的常见项目符号和短编号，避免把小数、版本号误作列表。
fn marker_prefix(text: &str) -> Option<&str> {
    let first = text.chars().next()?;
    if matches!(first, '•' | '●' | '◦' | '○' | '▪' | '‣' | '‧' | '・' | '·') {
        return Some(&text[..first.len_utf8()]);
    }
    if matches!(text, "-" | "–" | "*") {
        return Some(text);
    }
    let paren = first == '(';
    let start = usize::from(paren);
    let digits = text[start..].bytes().take_while(u8::is_ascii_digit).count();
    if !(1..=3).contains(&digits) {
        return None;
    }
    let delimiter = text[start + digits..].chars().next()?;
    if (paren && delimiter != ')') || (!paren && !matches!(delimiter, '.' | ')' | '、' | '．')) {
        return None;
    }
    let end = start + digits + delimiter.len_utf8();
    let tail = &text[end..];
    if delimiter == '.' && tail.starts_with(|c: char| !c.is_uppercase() && !is_cjk_char(c)) {
        return None;
    }
    Some(&text[..end])
}

/// 首个编号可能只识别到句点。只有后面紧跟对齐的 2.、3.，且句点左侧有数字笔画才补回。
fn recover_first_number(segments: &mut [Segment], image: &RgbaImage) {
    for i in 0..segments.len() {
        let s = &segments[i];
        let Some(body_x) = s.orphan_dot_body_x else { continue };
        let aligned = |n: &Segment| {
            let x = n.marker.as_ref().map_or(n.rect.x, |m| m.body_x);
            (x - body_x).abs() <= s.em * 0.35
        };
        let mut next = segments[i + 1..].iter()
            .take_while(|n| n.rect.y - s.rect.y <= 5.0 * s.em)
            .filter(|n| n.rect.y - s.rect.y >= 0.6 * s.em && aligned(n));
        let (Some(second), Some(third)) = (next.next(), next.next()) else { continue };
        let (Some(two), Some(three)) = (&second.marker, &third.marker) else { continue };
        let pitch = second.rect.y - s.rect.y;
        if two.text != "2." || three.text != "3."
            || (third.rect.y - second.rect.y - pitch).abs() > s.em * 0.45
            || (two.x - three.x).abs() > s.em * 0.25
            || !(s.em * 0.2..=s.em * 0.9).contains(&(s.rect.x - two.x))
        {
            continue;
        }
        let left = (two.x - s.em * 0.15).max(0.0) as u32;
        let right = (s.rect.x - 1.0).min(image.width() as f32) as u32;
        let top = s.rect.y.max(0.0) as u32;
        let bottom = s.rect.bottom().ceil().min(image.height() as f32) as u32;
        let threshold = (dist(s.bg, s.fg) / 2).max(60);
        let mut ink_left = right;
        let mut rows_with_ink = 0;
        for y in top..bottom {
            if let Some(x) = (left..right).find(|&x| {
                let [r, g, b, _] = image.get_pixel(x, y).0;
                dist(s.bg, [r, g, b]) >= threshold
            }) {
                ink_left = ink_left.min(x);
                rows_with_ink += 1;
            }
        }
        if (rows_with_ink as f32) < s.em * 0.4 {
            continue;
        }
        let x = ink_left as f32;
        let s = &mut segments[i];
        s.marker = Some(LineMarker { text: "1.".into(), body_x, x });
        s.text = s.text[1..].trim_start().to_string();
        s.rect.w = s.rect.right() - x;
        s.rect.x = x;
    }
}

/// OCR 偶尔忽略独立的实心圆点。只在行首左侧有隔开的紧凑墨点时补回，
/// 不根据缩进凭空补标记，以免把普通续行当成新项目。
fn recover_bullets(segments: &mut [Segment], image: &RgbaImage) {
    for s in segments {
        if s.marker.is_some() || !s.text.chars().any(char::is_alphabetic) {
            continue;
        }
        let body_x = s.rect.x;
        let left = (body_x - s.em * 1.6).max(0.0) as u32;
        let right = (body_x - s.em * 0.25).max(0.0) as u32;
        let top = s.rect.y.max(0.0) as u32;
        let bottom = s.rect.bottom().ceil().min(image.height() as f32) as u32;
        let threshold = (dist(s.bg, s.fg) / 2).max(60);
        let (mut x0, mut y0, mut x1, mut y1, mut count) = (right, bottom, left, top, 0);
        for y in top..bottom {
            for x in left..right.min(image.width()) {
                let [r, g, b, _] = image.get_pixel(x, y).0;
                if dist(s.bg, [r, g, b]) >= threshold {
                    x0 = x0.min(x);
                    x1 = x1.max(x + 1);
                    y0 = y0.min(y);
                    y1 = y1.max(y + 1);
                    count += 1;
                }
            }
        }
        if count == 0 || x0 == left || x1 == right || y0 == top || y1 == bottom {
            continue;
        }
        let w = (x1 - x0) as f32;
        let h = (y1 - y0) as f32;
        let mid = (y0 + y1) as f32 / 2.0;
        if !(s.em * 0.16..=s.em * 0.50).contains(&w)
            || !(0.7..=1.4).contains(&(w / h))
            || (count as f32) < w * h * 0.65
            || (mid - (s.rect.y + s.rect.h / 2.0)).abs() > s.em * 0.22
        {
            continue;
        }
        let x = x0 as f32;
        s.marker = Some(LineMarker { text: "•".into(), body_x, x });
        s.rect.w = s.rect.right() - x;
        s.rect.x = x;
    }
}

/// 截图没有标题标签。短行须比邻近正文短且笔画明显更粗，才当作独立标题。
fn mark_headings(segments: &mut [Segment], image: &RgbaImage, cjk: bool) {
    let ink = |s: &Segment| {
        stroke_ratio(stroke_width(image, &s.rect, s.bg, s.fg), s.em, luma(s.fg) > luma(s.bg))
    };
    for i in 0..segments.len() {
        let s = &segments[i];
        let count = s.text.chars().count();
        if s.marker.is_some() || count > 52 || s.text.split_whitespace().count() > 8
            || s.text.ends_with(['.', ',', ';', '!', '?', '。', '，', '；', '！', '？', '-'])
            || (cjk && count > 24)
        {
            continue;
        }
        let Some(next) = segments[i + 1..].iter()
            .take_while(|n| n.rect.y - s.rect.y <= 2.4 * s.em)
            .find(|n| n.rect.y - s.rect.y >= 0.6 * s.em
                && (n.rect.x - s.rect.x).abs() <= 0.55 * s.em)
        else { continue };
        if next.rect.w < s.rect.w * 1.25 || !next.text.chars().any(char::is_alphabetic) {
            continue;
        }
        let (own_ink, next_ink) = (ink(s), ink(next));
        if own_ink >= 0.09 && next_ink >= 0.04 && own_ink >= next_ink * 1.17 {
            segments[i].heading = true;
        }
    }
}

/// 拼接单词：中日文字符之间不加空格
fn push_word(out: &mut String, word: &str, cjk: bool) {
    let wide = |c: char| !c.is_ascii();
    let need_space = !out.is_empty()
        && !(cjk
            && out.chars().last().is_some_and(wide)
            && word.chars().next().is_some_and(wide));
    if need_space {
        out.push(' ');
    }
    out.push_str(word);
}

/// 段内多行合并成一段：英文行尾连字符断词时去掉连字符
fn join_lines<'a>(lines: impl Iterator<Item = &'a str>, cjk: bool) -> String {
    let mut out = String::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if out.ends_with('-') && line.chars().next().is_some_and(char::is_lowercase) {
            out.pop();
            out.push_str(line);
        } else {
            push_word(&mut out, line, cjk);
        }
    }
    out
}

/// 兼容测试入口：调用统一过滤模块判断是否需要翻译
#[cfg(test)]
fn needs_translation(text: &str, skip_cjk: bool) -> bool {
    let mut cfg = crate::config::Config::default();
    cfg.target_lang = if skip_cjk { "zh-CN".into() } else { "en".into() };
    let ctx = crate::filter::FilterContext {
        target_lang: &cfg.target_lang,
        app_id: "",
        app_name: "",
        mode: crate::filter::Mode::Overlay,
    };
    !matches!(crate::filter::classify(&cfg, text, &ctx), crate::filter::Decision::Keep(_))
}

fn is_cjk_char(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF)
}

/// 由笔画宽度判断字重。实测（Segoe UI / Georgia，12~28px）笔画宽度 / 字号：
/// 浅底深字：常规体 ≈ 0.08~0.09，半粗 ≈ 0.11，粗体 ≈ 0.15~0.17；
/// 深底浅字的抗锯齿会让笔画显得更粗，同字重的比例约为浅底的 1.25 倍，先折算回来。
fn stroke_ratio(stroke: f32, em: f32, light_on_dark: bool) -> f32 {
    stroke / em.max(1.0) / if light_on_dark { 1.25 } else { 1.0 }
}

fn weight_from_stroke(stroke: f32, em: f32, light_on_dark: bool) -> u16 {
    let ratio = stroke_ratio(stroke, em, light_on_dark);
    if ratio > 0.13 {
        700
    } else if ratio > 0.10 {
        600
    } else {
        400
    }
}

/// 根据单词由哪些字母组成，推算其框高占字号（em）的比例。
/// 例如 "mean" 只有 x 高度（≈0.52em），"The" 到大写高度（≈0.73em），"lightly" 上下都伸出（≈0.94em）。
/// 含括号等高度不确定的符号时返回 None。
fn height_ratio(word: &str, cjk: bool) -> Option<f32> {
    let word = word.trim_matches(|c: char| matches!(c, '.' | ',' | ':' | ';' | '!' | '?' | '"' | '\u{201c}' | '\u{201d}'));
    if word.is_empty() {
        return None;
    }
    if word.chars().any(is_cjk_char) || (cjk && word.chars().any(|c| !c.is_ascii())) {
        // 中日文字形基本占满字身框
        return Some(0.88);
    }
    let (mut asc, mut desc) = (false, false);
    for c in word.chars() {
        match c {
            'a' | 'c' | 'e' | 'm' | 'n' | 'o' | 'r' | 's' | 'u' | 'v' | 'w' | 'x' | 'z' | '-' | '\u{2013}' => {}
            'g' | 'p' | 'q' | 'y' => desc = true,
            'j' => {
                asc = true;
                desc = true;
            }
            'A'..='Z' | '0'..='9' | 'b' | 'd' | 'f' | 'h' | 'i' | 'k' | 'l' | 't' | '\'' | '\u{2019}' | '%' | '&' | '#' => {
                asc = true
            }
            _ => return None,
        }
    }
    Some(match (asc, desc) {
        (true, true) => 0.93,
        (true, false) => 0.73,
        (false, true) => 0.72,
        (false, false) => 0.52,
    })
}

/// 估算段落原文字号（em，物理像素）：逐词按字母组成换算后取中位数
fn estimate_font(group: &[Segment], line_h: f32, cjk: bool) -> f32 {
    let mut ems: Vec<f32> = group
        .iter()
        .flat_map(|s| &s.words)
        .filter_map(|(t, h)| height_ratio(t, cjk).map(|r| h / r))
        .collect();
    if ems.is_empty() {
        // 没有可用的单词：按"整行框高 ≈ 0.9em"估
        return line_h / 0.9;
    }
    ems.sort_by(f32::total_cmp);
    let em = ems[ems.len() / 2];
    // 防止个别识别异常把字号带偏太多
    em.clamp(line_h * 0.9, line_h * 2.0)
}

/// 多行段落的行距：相邻行顶部间距的中位数
fn line_pitch(group: &[Segment]) -> f32 {
    let mut d: Vec<f32> = group.windows(2).map(|w| w[1].rect.y - w[0].rect.y).filter(|d| *d > 0.0).collect();
    if d.is_empty() {
        return 0.0;
    }
    d.sort_by(f32::total_cmp);
    d[d.len() / 2]
}

/// 多行段落：各行左边缘参差、但中心对齐时判为居中
fn detect_align(group: &[Segment]) -> &'static str {
    if group.len() < 2 {
        return "left";
    }
    let spread = |v: Vec<f32>| {
        let lo = v.iter().cloned().fold(f32::MAX, f32::min);
        let hi = v.iter().cloned().fold(f32::MIN, f32::max);
        hi - lo
    };
    let h = group[0].rect.h;
    let lefts = spread(group.iter().map(|s| s.rect.x).collect());
    let centers = spread(group.iter().map(|s| s.rect.x + s.rect.w / 2.0).collect());
    if lefts > h && centers < h * 0.5 { "center" } else { "left" }
}

/// 单行文字：从块的左右边缘沿水平方向往外找第一个与背景色明显不同的像素（按钮边框、色块边界），
/// 两侧留白基本对称时说明文字居中放在按钮/标签里，译文也居中
fn boxed_align(image: &RgbaImage, cover: &Rect, bg: Rgb, em: f32) -> &'static str {
    let (iw, ih) = image.dimensions();
    let max_scan = (em * 3.0).max(8.0) as i32;
    let rows = [0.3f32, 0.5, 0.7].map(|t| (cover.y + cover.h * t) as i32);
    // 返回该方向上到边界的距离；超出扫描范围或图像边缘视为没有边界
    let scan = |from: i32, dir: i32| -> Option<i32> {
        let mut found = Vec::new();
        for &y in &rows {
            if y < 0 || y >= ih as i32 {
                return None;
            }
            let hit = (1..=max_scan).find(|&d| {
                let x = from + dir * d;
                x >= 0 && x < iw as i32 && {
                    let p = image.get_pixel(x as u32, y as u32).0;
                    dist([p[0], p[1], p[2]], bg) > 60
                }
            })?;
            found.push(hit);
        }
        found.sort_unstable();
        Some(found[1])
    };
    let left = scan(cover.x as i32, -1);
    let right = scan((cover.x + cover.w) as i32 - 1, 1);
    match (left, right) {
        (Some(l), Some(r)) if (l - r).abs() as f32 <= (l.max(r) as f32 * 0.3).max(3.0) => "center",
        _ => "left",
    }
}

/// 估算笔画宽度（物理像素，可带小数）：每个像素按"离背景色多远"折算成覆盖度（抗锯齿边缘算半个像素），
/// 一行里连续的有墨像素构成一段，段内覆盖度之和即该处笔画宽度，取所有段的中位数
fn stroke_width(image: &RgbaImage, r: &Rect, bg: Rgb, fg: Rgb) -> f32 {
    let full = dist(bg, fg) as f32;
    if full < 60.0 {
        return 0.0;
    }
    let (iw, ih) = image.dimensions();
    let x0 = (r.x.max(0.0) as u32).min(iw);
    let y0 = (r.y.max(0.0) as u32).min(ih);
    let x1 = (r.right().ceil() as u32).min(iw);
    let y1 = (r.bottom().ceil() as u32).min(ih);
    let mut runs: Vec<f32> = Vec::new();
    for y in y0..y1 {
        let mut run = 0.0f32;
        let mut peak = 0.0f32;
        for x in x0..=x1 {
            let a = if x < x1 {
                let p = image.get_pixel(x, y).0;
                (dist([p[0], p[1], p[2]], bg) as f32 / full).min(1.0)
            } else {
                0.0
            };
            if a > 0.15 {
                run += a;
                peak = peak.max(a);
            } else if run > 0.0 {
                // 只统计真正压到文字色的段，过滤掉浅色噪点
                if peak > 0.6 {
                    runs.push(run);
                }
                run = 0.0;
                peak = 0.0;
            }
        }
    }
    if runs.len() < 8 {
        return 0.0;
    }
    runs.sort_by(f32::total_cmp);
    runs[runs.len() / 2]
}

type Rgb = [u8; 3];

/// 背景色 = 外框一圈像素的中位数；文字色 = 框内笔画核心（与背景差异接近最大）像素的平均值
fn sample_colors(image: &RgbaImage, r: &Rect) -> (Rgb, Rgb) {
    let (iw, ih) = image.dimensions();
    let x0 = (r.x.max(0.0) as u32).min(iw.saturating_sub(1));
    let y0 = (r.y.max(0.0) as u32).min(ih.saturating_sub(1));
    let x1 = (r.right().ceil() as u32).clamp(x0 + 1, iw);
    let y1 = (r.bottom().ceil() as u32).clamp(y0 + 1, ih);
    let px = |x: u32, y: u32| {
        let p = image.get_pixel(x, y).0;
        [p[0], p[1], p[2]]
    };

    let mut border = Vec::new();
    let step = ((x1 - x0) / 64).max(1);
    for x in (x0..x1).step_by(step as usize) {
        border.push(px(x, y0));
        border.push(px(x, y1 - 1));
    }
    let step = ((y1 - y0) / 32).max(1);
    for y in (y0..y1).step_by(step as usize) {
        border.push(px(x0, y));
        border.push(px(x1 - 1, y));
    }
    let bg = median(&mut border);

    // 内部采样。细字的大部分"墨"像素是抗锯齿后的浅色边缘，平均进来会让文字色发灰，
    // 所以只取笔画核心：与背景差异达到"近似最大差异"85% 以上的像素
    let mut inner: Vec<(u32, Rgb)> = Vec::new();
    let sx = ((x1 - x0) / 200).max(1);
    let sy = ((y1 - y0) / 60).max(1);
    for y in (y0..y1).step_by(sy as usize) {
        for x in (x0..x1).step_by(sx as usize) {
            let c = px(x, y);
            inner.push((dist(c, bg), c));
        }
    }
    inner.sort_by(|a, b| b.0.cmp(&a.0));
    // 取第 1 百分位作为"最大差异"，避开个别噪点
    let peak = inner.get(inner.len() / 100).map_or(0, |p| p.0);
    let take = inner.iter().take_while(|p| p.0 * 100 >= peak * 85).count().max(1).min(inner.len());
    let fg = if take == 0 || peak < 40 {
        // 几乎没有对比：按背景亮度选黑或白
        if luma(bg) > 128 { [0, 0, 0] } else { [255, 255, 255] }
    } else {
        let mut sum = [0u32; 3];
        for (_, c) in &inner[..take] {
            for i in 0..3 {
                sum[i] += c[i] as u32;
            }
        }
        [0, 1, 2].map(|i| (sum[i] / take as u32) as u8)
    };
    (bg, fg)
}

fn median(colors: &mut [Rgb]) -> Rgb {
    if colors.is_empty() {
        return [255, 255, 255];
    }
    let mid = colors.len() / 2;
    [0, 1, 2].map(|i| {
        colors.sort_by_key(|c| c[i]);
        colors[mid][i]
    })
}

fn dist(a: Rgb, b: Rgb) -> u32 {
    (0..3).map(|i| (a[i] as i32 - b[i] as i32).unsigned_abs()).sum()
}

fn luma(c: Rgb) -> u32 {
    (c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114) / 1000
}

fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 构造单词：框高按字母组成换算（字号 18px），与真实 OCR 结果一致
    fn word(text: &str, x: f32, y: f32, w: f32) -> Word {
        let h = height_ratio(text, false).unwrap_or(0.93) * 18.0;
        Word {
            text: text.into(),
            x,
            y: y + 0.93 * 18.0 - h,
            w,
            h,
        }
    }

    #[test]
    fn 段落归并与分栏() {
        // 左栏两行属于同一段；右栏与左栏间距大，单独成段；下方隔得远的另起一段
        let ocr = OcrOutput {
            cjk: false,
            lines: vec![
                Line {
                    words: vec![word("Hello", 10.0, 10.0, 50.0), word("wor-", 65.0, 10.0, 40.0), word("Right", 300.0, 10.0, 50.0)],
                },
                Line {
                    words: vec![word("ld", 10.0, 30.0, 20.0), word("again.", 35.0, 30.0, 60.0)],
                },
                Line {
                    words: vec![word("Next", 10.0, 120.0, 40.0)],
                },
            ],
        };
        let img = RgbaImage::from_pixel(400, 200, Rgba([255, 255, 255, 255]));
        let blocks = build_blocks(&ocr, &img, true);
        let mut texts: Vec<_> = blocks.iter().map(|b| b.source.as_str()).collect();
        texts.sort();
        assert_eq!(texts, ["Hello world again.", "Next", "Right"]);
        let first = blocks.iter().find(|b| b.source.starts_with("Hello")).unwrap();
        assert_eq!(first.lines, 2);
        assert_eq!(first.bg, "#ffffff");
    }

    #[test]
    fn 列表标记独立保留且续行归入原项目() {
        let ocr = OcrOutput {
            cjk: false,
            lines: vec![
                Line { words: vec![word("•", 10.0, 10.0, 6.0), word("Open Settings", 30.0, 10.0, 110.0)] },
                Line { words: vec![word("and choose a language.", 30.0, 31.0, 165.0)] },
                Line { words: vec![word("2.Save changes", 10.0, 52.0, 130.0)] },
                Line { words: vec![word("Separate paragraph.", 10.0, 73.0, 160.0)] },
            ],
        };
        let img = RgbaImage::from_pixel(300, 120, Rgba([255, 255, 255, 255]));
        let blocks = build_blocks(&ocr, &img, false);
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].source, "Open Settings and choose a language.");
        assert_eq!(blocks[0].lines, 2);
        let marker = blocks[0].marker.as_ref().unwrap();
        assert_eq!(marker.text, "•");
        assert_eq!(blocks[0].x + marker.offset, 10.0);
        assert_eq!(blocks[0].x + marker.indent, 30.0);
        assert_eq!(blocks[1].source, "Save changes");
        assert_eq!(blocks[1].marker.as_ref().unwrap().text, "2.");
        assert_eq!(blocks[2].source, "Separate paragraph.");
        assert!(blocks[2].marker.is_none());
    }

    #[test]
    fn 只补回有独立墨点的项目符号() {
        let ocr = OcrOutput {
            cjk: false,
            lines: vec![
                Line { words: vec![word("First item", 34.0, 10.0, 95.0)] },
                Line { words: vec![word("with a wrapped line.", 34.0, 31.0, 140.0)] },
                Line { words: vec![word("Next item", 34.0, 52.0, 90.0)] },
            ],
        };
        let mut img = RgbaImage::from_pixel(300, 100, Rgba([255, 255, 255, 255]));
        for offset in [0, 42] {
            for y in 18 + offset..24 + offset {
                for x in 16..22 {
                    img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
                }
            }
        }
        let blocks = build_blocks(&ocr, &img, false);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].source, "First item with a wrapped line.");
        assert_eq!(blocks[1].source, "Next item");
        for block in blocks {
            let marker = block.marker.unwrap();
            assert_eq!(marker.text, "•");
            assert_eq!(block.x + marker.offset, 16.0);
            assert_eq!(block.x + marker.indent, 34.0);
        }
    }

    #[test]
    fn 首个编号需相邻序号和残留笔画才补回() {
        let ocr = OcrOutput {
            cjk: false,
            lines: vec![
                Line { words: vec![word(".", 45.0, 10.0, 3.0), word("Open Settings", 54.0, 10.0, 100.0)] },
                Line { words: vec![word("2.", 36.0, 31.0, 12.0), word("Choose a language", 54.0, 31.0, 130.0)] },
                Line { words: vec![word("3.", 36.0, 52.0, 12.0), word("Save changes", 54.0, 52.0, 100.0)] },
            ],
        };
        let mut img = RgbaImage::from_pixel(300, 100, Rgba([255, 255, 255, 255]));
        let blocks = build_blocks(&ocr, &img, false);
        assert!(blocks[0].marker.is_none());
        assert_eq!(blocks[0].source, ". Open Settings");

        for y in 14..26 {
            img.put_pixel(37, y, Rgba([0, 0, 0, 255]));
        }
        let blocks = build_blocks(&ocr, &img, false);
        assert_eq!(blocks[0].source, "Open Settings");
        let marker = blocks[0].marker.as_ref().unwrap();
        assert_eq!(marker.text, "1.");
        assert_eq!(blocks[0].x + marker.offset, 37.0);
        assert_eq!(blocks[0].x + marker.indent, 54.0);
    }

    #[test]
    fn 小数和版本号保留在正文() {
        for text in ["3.14", "2.4.1", "2027.", "v2.0", "1.em"] {
            assert_eq!(marker_prefix(text), None, "{text}");
        }
        assert_eq!(marker_prefix("2)"), Some("2)"));
        assert_eq!(marker_prefix("(3)"), Some("(3)"));
        assert_eq!(marker_prefix("4、下一步"), Some("4、"));
    }

    #[test]
    fn 粗体标题独立且正文自然换行仍合并() {
        let ocr = OcrOutput {
            cjk: false,
            lines: vec![
                Line { words: vec![word("Important Terms", 10.0, 10.0, 105.0)] },
                Line { words: vec![word("Users can cancel before the trial ends", 10.0, 31.0, 250.0)] },
                Line { words: vec![word("to avoid being charged.", 10.0, 52.0, 155.0)] },
            ],
        };
        let mut img = RgbaImage::from_pixel(300, 100, Rgba([255, 255, 255, 255]));
        for (x_end, top, thickness) in [(115, 14, 3), (260, 35, 1), (165, 56, 1)] {
            for y in top..top + 10 {
                for x in (10..x_end).step_by(7) {
                    for dx in 0..thickness {
                        img.put_pixel(x + dx, y, Rgba([0, 0, 0, 255]));
                    }
                }
            }
        }
        let blocks = build_blocks(&ocr, &img, false);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].source, "Important Terms");
        assert_eq!(blocks[0].weight, 700);
        assert_eq!(blocks[1].source, "Users can cancel before the trial ends to avoid being charged.");
        assert_eq!(blocks[1].lines, 2);
    }

    #[test]
    fn 跳过无需翻译的块() {
        assert!(!needs_translation("123 - 456", false));
        assert!(!needs_translation("已经是中文了", true));
        assert!(needs_translation("Hello", true));
    }

    #[test]
    fn 取样颜色() {
        let mut img = RgbaImage::from_pixel(100, 40, Rgba([30, 30, 30, 255]));
        for x in 20..80 {
            for y in 15..25 {
                img.put_pixel(x, y, Rgba([240, 240, 240, 255]));
            }
        }
        let (bg, fg) = sample_colors(&img, &Rect { x: 0.0, y: 0.0, w: 100.0, h: 40.0 });
        assert_eq!(bg, [30, 30, 30]);
        assert_eq!(fg, [240, 240, 240]);
    }

    /// 真机校准（后台进行）：抓取窗口 TF_HWND 的画面裁出 TF_RECT（窗口内坐标），或读取图片 TF_IMG
    /// → OCR → 打印每段估算的字号/行距/笔画比；TF_JSON 指定时把版面存成 JSON。
    /// TF_HWND=句柄 TF_RECT=x,y,w,h cargo test 字号校准 -- --ignored --nocapture
    #[test]
    #[ignore]
    fn 字号校准() {
        let v: Vec<i32> = std::env::var("TF_RECT")
            .unwrap_or_else(|_| "0,0,900,900".into())
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        // TF_IMG：直接读已保存的截图；否则抓取后台窗口 TF_HWND
        let image = if let Ok(p) = std::env::var("TF_IMG") {
            image::open(p).unwrap().to_rgba8()
        } else {
            let hwnd: isize = std::env::var("TF_HWND").unwrap().parse().unwrap();
            let full = crate::capture::capture_window(hwnd).unwrap();
            image::imageops::crop_imm(&full, v[0] as u32, v[1] as u32, v[2] as u32, v[3] as u32).to_image()
        };
        if let Ok(p) = std::env::var("TF_SHOT") {
            image.save(p).unwrap();
        }
        let out = crate::ocr::recognize(&image, "auto", "zh-CN").unwrap();
        let blocks = build_blocks(&out, &image, true);
        if let Ok(p) = std::env::var("TF_JSON") {
            std::fs::write(p, serde_json::to_string_pretty(&blocks).unwrap()).unwrap();
        }
        for b in blocks {
            let rect = Rect { x: b.x, y: b.y, w: b.w, h: b.h };
            let bg = parse_hex(&b.bg);
            let fg = parse_hex(&b.fg);
            let stroke = stroke_width(&image, &rect, bg, fg);
            println!(
                "字号 {:5.1} 行距 {:5.1} 行高 {:5.1} 笔画 {:4.2} 比 {:.3} 字重 {} {} 色 {} | {}",
                b.font, b.pitch, b.line_h, stroke, stroke / b.font, b.weight, b.align, b.fg,
                b.source.chars().take(40).collect::<String>()
            );
        }
    }

    fn parse_hex(s: &str) -> Rgb {
        let v = u32::from_str_radix(&s[1..], 16).unwrap();
        [(v >> 16) as u8, (v >> 8) as u8, v as u8]
    }

    #[test]
    fn 按钮内文字居中() {
        // 蓝色按钮（20..180）里一行文字（50..150），左右留白对称
        let mut img = RgbaImage::from_pixel(240, 60, Rgba([255, 255, 255, 255]));
        for y in 10..50 {
            for x in 20..180 {
                img.put_pixel(x, y, Rgba([47, 140, 255, 255]));
            }
        }
        let bg = [47, 140, 255];
        let cover = Rect { x: 50.0, y: 20.0, w: 100.0, h: 20.0 };
        assert_eq!(boxed_align(&img, &cover, bg, 14.0), "center");
        // 文字靠左放：左留白 10、右留白 70，不对称
        let cover = Rect { x: 30.0, y: 20.0, w: 80.0, h: 20.0 };
        assert_eq!(boxed_align(&img, &cover, bg, 14.0), "left");
        // 白底上的普通一行字：两侧找不到边界
        let cover = Rect { x: 190.0, y: 5.0, w: 40.0, h: 4.0 };
        assert_eq!(boxed_align(&img, &cover, [255, 255, 255], 14.0), "left");
    }
}
