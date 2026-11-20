//! 画面变化检测与滚动位移估计。
//!
//! 滚动时 OCR 需要一两百毫秒，先用行/列"签名"快速估出内容整体平移了多少，
//! 让译文立即跟着平移，等 OCR 结果出来再精确校正。

use image::RgbaImage;

/// 每行（列）签名分成多少段
const BUCKETS: usize = 32;
/// 视为"有内容"的行签名最小起伏（灰度）
const MIN_ROW_VAR: f32 = 6.0;

/// 灰度帧
pub struct Frame {
    pub w: usize,
    pub h: usize,
    gray: Vec<u8>,
}

impl Frame {
    pub fn from_rgba(img: &RgbaImage) -> Self {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let gray = img
            .as_raw()
            .chunks_exact(4)
            .map(|p| ((p[0] as u32 * 77 + p[1] as u32 * 150 + p[2] as u32 * 29) >> 8) as u8)
            .collect();
        Self { w, h, gray }
    }

    fn at(&self, x: usize, y: usize) -> u8 {
        self.gray[y * self.w + x]
    }
}

/// 两帧之间发生明显变化的像素比例（稀疏采样）；尺寸不同视为完全变化
pub fn changed_fraction(a: &Frame, b: &Frame) -> f32 {
    if a.w != b.w || a.h != b.h || a.w == 0 || a.h == 0 {
        return 1.0;
    }
    let (mut total, mut changed) = (0u32, 0u32);
    for y in (0..a.h).step_by(3) {
        for x in (0..a.w).step_by(3) {
            total += 1;
            if a.at(x, y).abs_diff(b.at(x, y)) > 24 {
                changed += 1;
            }
        }
    }
    changed as f32 / total.max(1) as f32
}

/// 估计内容从 `prev` 到 `cur` 的整体平移（屏幕像素，向下/向右为正）；
/// 只检测单方向滚动，不是平移或不可靠时返回 None
pub fn estimate_shift(prev: &Frame, cur: &Frame) -> Option<(i32, i32)> {
    if prev.w != cur.w || prev.h != cur.h || cur.w < 8 || cur.h < 8 {
        return None;
    }
    let rows_prev = row_signatures(prev);
    let rows_cur = row_signatures(cur);
    if let Some(d) = best_offset(&rows_prev, &rows_cur) {
        return Some((0, d));
    }
    let cols_prev = col_signatures(prev);
    let cols_cur = col_signatures(cur);
    best_offset(&cols_prev, &cols_cur).map(|d| (d, 0))
}

type Sig = [f32; BUCKETS];

fn row_signatures(f: &Frame) -> Vec<Sig> {
    (0..f.h)
        .map(|y| {
            let mut s = [0.0; BUCKETS];
            for (i, v) in s.iter_mut().enumerate() {
                let (x0, x1) = (i * f.w / BUCKETS, ((i + 1) * f.w / BUCKETS).max(i * f.w / BUCKETS + 1));
                let sum: u32 = (x0..x1.min(f.w)).map(|x| f.at(x, y) as u32).sum();
                *v = sum as f32 / (x1 - x0) as f32;
            }
            s
        })
        .collect()
}

fn col_signatures(f: &Frame) -> Vec<Sig> {
    (0..f.w)
        .map(|x| {
            let mut s = [0.0; BUCKETS];
            for (i, v) in s.iter_mut().enumerate() {
                let (y0, y1) = (i * f.h / BUCKETS, ((i + 1) * f.h / BUCKETS).max(i * f.h / BUCKETS + 1));
                let sum: u32 = (y0..y1.min(f.h)).map(|y| f.at(x, y) as u32).sum();
                *v = sum as f32 / (y1 - y0) as f32;
            }
            s
        })
        .collect()
}

fn variance(s: &Sig) -> f32 {
    let mean = s.iter().sum::<f32>() / BUCKETS as f32;
    s.iter().map(|v| (v - mean).abs()).sum::<f32>() / BUCKETS as f32
}

fn sig_diff(a: &Sig, b: &Sig) -> f32 {
    a.iter().zip(b).map(|(x, y)| (x - y).abs()).sum::<f32>() / BUCKETS as f32
}

/// 粗搜时最多使用的有效行数（均匀抽样），精搜时再用全部有效行核对
const COARSE_ROWS: usize = 96;
/// 粗搜保留的候选偏移个数
const COARSE_CANDIDATES: usize = 4;
/// 截尾平均保留最吻合的这部分行：允许最多四成的行不随滚动（固定的标题栏、侧栏）
const TRIM_KEEP: f32 = 0.6;
/// 截尾平均低于该值才算"这部分行几乎完全对上"（滚动时像素原样平移，真实误差接近 0）
const TRIM_MAX_ERR: f32 = 1.5;

/// 在 [-n/2, n/2] 内找使 cur[i] ≈ prev[i + s] 的 s，返回内容位移 -s。
/// 先用抽样行粗搜所有偏移，再在最优偏移附近用全部有效行精确核对，耗时与画面高度近似线性。
fn best_offset(prev: &[Sig], cur: &[Sig]) -> Option<i32> {
    let n = cur.len() as i32;
    let max = n / 2;
    // 只用"有内容"的行比较，空白行和什么都能匹配
    let informative: Vec<usize> = (0..cur.len()).filter(|&i| variance(&cur[i]) > MIN_ROW_VAR).collect();
    if informative.len() < 4 {
        return None;
    }
    let step = informative.len().div_ceil(COARSE_ROWS);
    let sampled: Vec<usize> = informative.iter().step_by(step).copied().collect();
    // 每个偏移给出两种误差：全部行的平均（用来挑偏移，位置准）和最吻合那部分行的平均（截尾平均，用来判断可不可信）。
    // 框里有固定不动的标题栏、侧栏时，这些行在真实位移下对不上，只看全部平均会判为不可信、译文不再跟随滚动；
    // 但截尾平均会容忍几像素的错位（同一行文字上下几像素很像），不能用它挑偏移
    let err_at = |rows: &[usize], s: i32| -> Option<(f32, f32)> {
        let mut diffs: Vec<f32> = rows
            .iter()
            .filter_map(|&i| {
                let j = i as i32 + s;
                (j >= 0 && j < n).then(|| sig_diff(&cur[i], &prev[j as usize]))
            })
            .collect();
        // 重叠的有效行太少不可信
        if diffs.is_empty() || diffs.len() * 2 < rows.len() {
            return None;
        }
        let mean = diffs.iter().sum::<f32>() / diffs.len() as f32;
        let keep = ((diffs.len() as f32 * TRIM_KEEP).ceil() as usize).clamp(1, diffs.len());
        diffs.select_nth_unstable_by(keep - 1, f32::total_cmp);
        Some((mean, diffs[..keep].iter().sum::<f32>() / keep as f32))
    };

    // 粗搜：保留误差最小的几个候选（抽样可能被重复图案骗到，不能只信第一名）
    let mut candidates: Vec<(f32, i32)> = (-max..=max)
        .filter(|&s| s != 0)
        .filter_map(|s| err_at(&sampled, s).map(|e| (e.0, s)))
        .collect();
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
    candidates.truncate(COARSE_CANDIDATES);

    // 精搜：在每个候选 ±2 内用全部有效行核对，取全部平均最小的偏移
    let (mean0, trim0) = err_at(&informative, 0)?;
    let mut best: Option<(i32, f32, f32)> = None;
    for &(_, c) in &candidates {
        for s in (c - 2)..=(c + 2) {
            if s == 0 || s.abs() > max {
                continue;
            }
            if let Some((mean, trim)) = err_at(&informative, s) {
                if best.is_none_or(|b| mean < b.1) {
                    best = Some((s, mean, trim));
                }
            }
        }
    }
    let (best_s, mean, trim) = best?;
    // 必须明显优于"没动"，且本身足够吻合：整体都对得上，或者至少六成的行几乎完全对上
    let whole = mean < 3.0 && mean < mean0 * 0.4;
    let mostly = trim < TRIM_MAX_ERR && trim < trim0 * 0.4;
    if whole || mostly {
        Some(-best_s)
    } else {
        None
    }
}

/// 某块区域在两帧之间是"原地没动"还是"随内容平移"了
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RegionMotion {
    /// 原位置的画面没变（固定的标题栏、侧栏、视频字幕等）
    Static,
    /// 画面跟着整体位移走了
    Moved,
}

/// 区域对比时的采样步长（像素）
const REGION_STEP: usize = 2;
/// 原地误差低于该值（灰度平均差）才可能算没动
const STATIC_MAX_ERR: f32 = 6.0;

/// 比较 `a` 中矩形区域 (x, y, w, h) 与 `b` 中平移 (dx, dy) 后的同一区域，返回平均灰度差；
/// 有效采样不到区域的三成（大半移出画面）时返回 None
fn region_diff(a: &Frame, b: &Frame, (x, y, w, h): (i32, i32, i32, i32), (dx, dy): (i32, i32)) -> Option<f32> {
    let (fw, fh) = (a.w as i32, a.h as i32);
    let (mut sum, mut cnt, mut all) = (0u64, 0u32, 0u32);
    for py in (y..y + h).step_by(REGION_STEP) {
        for px in (x..x + w).step_by(REGION_STEP) {
            all += 1;
            let (qx, qy) = (px + dx, py + dy);
            if px < 0 || py < 0 || px >= fw || py >= fh || qx < 0 || qy < 0 || qx >= fw || qy >= fh {
                continue;
            }
            sum += a.at(px as usize, py as usize).abs_diff(b.at(qx as usize, qy as usize)) as u64;
            cnt += 1;
        }
    }
    (all > 0 && cnt * 10 >= all * 3).then(|| sum as f32 / cnt as f32)
}

/// 判断 `a` 中的一块区域到 `b` 时是原地不动还是随整体位移 `d` 平移了；分不清（没有纹理、移出画面）时返回 None。
///
/// 整体滚动是按全框估计的，框里如果有固定不动的部分（标题栏、侧栏、视频字幕），
/// 只按整体位移平移译文会让这些地方的译文"漂"走，所以逐块核对。
pub fn classify_region(a: &Frame, b: &Frame, rect: (i32, i32, i32, i32), d: (i32, i32)) -> Option<RegionMotion> {
    if a.w != b.w || a.h != b.h || d == (0, 0) || rect.2 < 2 || rect.3 < 2 {
        return None;
    }
    let still = region_diff(a, b, rect, (0, 0))?;
    let moved = region_diff(a, b, rect, d)?;
    // 必须明显更吻合其中一种，才下结论
    if still < STATIC_MAX_ERR && moved > still * 2.0 + 4.0 {
        Some(RegionMotion::Static)
    } else if moved < STATIC_MAX_ERR && still > moved * 2.0 + 4.0 {
        Some(RegionMotion::Moved)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 生成一张有若干"文字行"的图，`offset` 为内容向下的位移
    fn page(offset: i32) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(200, 300, Rgba([255, 255, 255, 255]));
        for line in 0..20 {
            let top = line * 24 + offset;
            for y in top..top + 10 {
                if !(0..300).contains(&y) {
                    continue;
                }
                for x in 0..200 {
                    // 每行不同的"字形"图案
                    if (x / (3 + line as u32 % 5) + line as u32) % 3 == 0 {
                        img.put_pixel(x, y as u32, Rgba([0, 0, 0, 255]));
                    }
                }
            }
        }
        img
    }

    #[test]
    fn 检测垂直滚动() {
        let a = Frame::from_rgba(&page(0));
        let b = Frame::from_rgba(&page(-37));
        assert_eq!(estimate_shift(&a, &b), Some((0, -37)));
        let c = Frame::from_rgba(&page(13));
        assert_eq!(estimate_shift(&a, &c), Some((0, 13)));
    }

    #[test]
    fn 未变化() {
        let a = Frame::from_rgba(&page(0));
        let b = Frame::from_rgba(&page(0));
        assert_eq!(changed_fraction(&a, &b), 0.0);
        assert_eq!(estimate_shift(&a, &b), None);
    }

    #[test]
    fn 位移估计耗时() {
        // 模拟 900x1400 的大框
        let mk = |offset: i32| {
            let mut img = RgbaImage::from_pixel(900, 1400, Rgba([255, 255, 255, 255]));
            for line in 0..90 {
                let top = line * 16 + offset;
                for y in top..top + 9 {
                    if !(0..1400).contains(&y) {
                        continue;
                    }
                    for x in 0..900 {
                        // 每行不同的伪随机"字形"
                        let h = (x / 3u32).wrapping_mul(2654435761u32).wrapping_add((y - top) as u32 * 7919 + line as u32 * 40503) >> 13;
                        if h % 3 == 0 {
                            img.put_pixel(x, y as u32, Rgba([0, 0, 0, 255]));
                        }
                    }
                }
            }
            img
        };
        let (a, b) = (mk(0), mk(-23));
        let t = std::time::Instant::now();
        let (fa, fb) = (Frame::from_rgba(&a), Frame::from_rgba(&b));
        println!("灰度化 {:?}", t.elapsed());
        let t2 = std::time::Instant::now();
        let _ = row_signatures(&fa);
        println!("行签名 {:?}", t2.elapsed());
        let t2 = std::time::Instant::now();
        let _ = changed_fraction(&fa, &fb);
        println!("变化比例 {:?}", t2.elapsed());
        let d = estimate_shift(&fa, &fb);
        let el = t.elapsed();
        println!("灰度化 + 位移估计耗时 {el:?}");
        assert_eq!(d, Some((0, -23)));
    }

    /// 上半部分固定（标题栏）、下半部分滚动的画面
    fn split_page(offset: i32) -> RgbaImage {
        let fixed = page(0);
        let moving = page(offset);
        let mut img = moving.clone();
        for y in 0..50 {
            for x in 0..200 {
                img.put_pixel(x, y, *fixed.get_pixel(x, y));
            }
        }
        img
    }

    #[test]
    fn 逐块区分固定与滚动的区域() {
        let a = Frame::from_rgba(&split_page(0));
        let b = Frame::from_rgba(&split_page(-24));
        // 整体估计出向上滚了 24 像素
        assert_eq!(estimate_shift(&a, &b), Some((0, -24)));
        // 固定的标题栏里的一行文字（第 1 行，y 24..34）
        assert_eq!(classify_region(&a, &b, (0, 22, 200, 14), (0, -24)), Some(RegionMotion::Static));
        // 滚动区域里的一行文字（第 8 行，y 192..202）
        assert_eq!(classify_region(&a, &b, (0, 190, 200, 14), (0, -24)), Some(RegionMotion::Moved));
        // 空白处分不清
        assert_eq!(classify_region(&a, &b, (0, 12, 200, 10), (0, -24)), None);
    }
}
