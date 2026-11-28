//! 各环节耗时日志：只在 debug 构建输出到控制台，release 下整段被编译器去掉

use std::time::Duration;

/// 打一行耗时日志（前缀“[耗时]”，便于在控制台里筛选）
macro_rules! tlog {
    ($($arg:tt)*) => {
        if cfg!(debug_assertions) {
            println!("[耗时] {}", format_args!($($arg)*));
        }
    };
}

/// 秒，保留两位小数
pub fn secs(d: Duration) -> String {
    format!("{:.2} s", d.as_secs_f64())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 耗时格式化为两位小数秒() {
        assert_eq!(secs(Duration::from_millis(0)), "0.00 s");
        assert_eq!(secs(Duration::from_millis(684)), "0.68 s");
        assert_eq!(secs(Duration::from_millis(1236)), "1.24 s");
    }
}
