//! 极简结构化日志：每条一行 JSON，写入 `%LOCALAPPDATA%\disk-cleaner-rs\logs\app.log`。
//!
//! 不引入第三方依赖。发布版是 GUI 子系统、没有控制台，日志文件是唯一的现场取证手段；
//! 初始化或写入失败时静默降级，绝不影响主流程。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// 单文件上限 5 MiB，超过后滚动，最多保留 3 个历史文件（合计约 20 MiB）。
const MAX_BYTES: u64 = 5 * 1024 * 1024;
const KEEP_FILES: usize = 3;

#[derive(Clone, Copy)]
pub(crate) enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    fn as_str(self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

pub(crate) enum Value<'a> {
    Str(&'a str),
    Num(u64),
}

impl Value<'_> {
    fn to_json(&self) -> String {
        match self {
            Value::Str(text) => format!("\"{}\"", json_escape(text)),
            Value::Num(number) => number.to_string(),
        }
    }
}

struct Logger {
    path: PathBuf,
    inner: Mutex<(File, u64)>,
}

static LOGGER: OnceLock<Option<Logger>> = OnceLock::new();

/// 初始化日志。重复调用无副作用；失败时静默降级为无操作。
pub(crate) fn init() {
    LOGGER.get_or_init(|| match Logger::open() {
        Ok(logger) => Some(logger),
        Err(error) => {
            eprintln!("日志初始化失败：{error}");
            None
        }
    });
}

/// 写入一条事件。`fields` 是键值对，值类型由 [`Value`] 决定。
pub(crate) fn event(level: Level, name: &str, fields: &[(&str, Value<'_>)]) {
    let Some(logger) = LOGGER.get().and_then(|slot| slot.as_ref()) else {
        return;
    };
    let mut line = String::with_capacity(256);
    line.push_str("{\"ts\":\"");
    line.push_str(&iso8601_utc(SystemTime::now()));
    line.push_str("\",\"level\":\"");
    line.push_str(level.as_str());
    line.push_str("\",\"event\":\"");
    line.push_str(&json_escape(name));
    line.push_str("\",\"pid\":");
    line.push_str(&std::process::id().to_string());
    for (key, value) in fields {
        line.push_str(",\"");
        line.push_str(&json_escape(key));
        line.push_str("\":");
        line.push_str(&value.to_json());
    }
    line.push('}');
    line.push('\n');
    logger.write(&line);
}

impl Logger {
    fn open() -> std::io::Result<Self> {
        let path = log_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata().map(|meta| meta.len()).unwrap_or(0);
        Ok(Self {
            path,
            inner: Mutex::new((file, written)),
        })
    }

    fn write(&self, line: &str) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        let (file, written) = &mut *guard;
        if *written + line.len() as u64 > MAX_BYTES {
            self.rotate(file, written);
        }
        if let Err(error) = file.write_all(line.as_bytes()) {
            eprintln!("写日志失败：{error}");
            return;
        }
        *written += line.len() as u64;
    }

    /// 滚动规则：`app.log.2 → app.log.3`、`app.log.1 → app.log.2`、`app.log → app.log.1`。
    fn rotate(&self, file: &mut File, written: &mut u64) {
        for index in (1..KEEP_FILES).rev() {
            let _ = std::fs::rename(numbered(&self.path, index), numbered(&self.path, index + 1));
        }
        let _ = std::fs::rename(&self.path, numbered(&self.path, 1));
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            Ok(fresh) => *file = fresh,
            Err(error) => eprintln!("日志滚动失败：{error}"),
        }
        *written = 0;
    }
}

fn log_path() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("disk-cleaner-rs").join("logs").join("app.log")
}

fn numbered(path: &Path, index: usize) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{index}"));
    path.with_file_name(name)
}

/// 转成 `YYYY-MM-DDTHH:MM:SS.mmmZ`，不依赖 chrono。
fn iso8601_utc(time: SystemTime) -> String {
    let elapsed = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let seconds = elapsed.as_secs();
    let millis = elapsed.subsec_millis();
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        rest / 3_600,
        (rest % 3_600) / 60,
        rest % 60
    )
}

/// Howard Hinnant 的 days → civil date 算法，处理 1970 年前后与闰年。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

/// 转义 JSON 字符串：引号、反斜杠、换行、制表符与控制字符。
fn json_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            control if (control as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", control as u32));
            }
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn escapes_json_control_characters() {
        assert_eq!(json_escape("a\"b\\c"), "a\\\"b\\\\c");
        assert_eq!(json_escape("行1\n行2\t"), "行1\\n行2\\t");
        assert_eq!(json_escape("x\u{1}y"), "x\\u0001y");
    }

    #[test]
    fn formats_utc_timestamp() {
        assert_eq!(iso8601_utc(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        assert_eq!(
            iso8601_utc(UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            "2023-11-14T22:13:20.000Z"
        );
    }
}
