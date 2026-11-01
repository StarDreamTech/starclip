//! Clipboard transition bookkeeping, tested without touching the system clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureKey {
    Text(String),
    Image(String),
    /// 换行连接的文件绝对路径。
    Files(String),
}

#[derive(Default)]
pub struct CaptureState {
    sequence: Option<u64>,
    previous: Option<CaptureKey>,
}
impl CaptureState {
    pub fn should_read(&self, sequence: Option<u64>) -> bool {
        // Starting/resuming resets this state, so the current clipboard is read
        // once. Only a successful read (or an invalid item) consumes a sequence.
        sequence.is_none() || self.sequence != sequence
    }
    pub fn needs_capture(&self, key: &Option<CaptureKey>) -> bool {
        key.is_some() && self.previous != *key
    }
    pub fn accept(&mut self, sequence: Option<u64>, key: Option<CaptureKey>) {
        self.sequence = sequence;
        self.previous = key;
    }
    pub fn skip_invalid(&mut self, sequence: Option<u64>) {
        // Don't decode/encode an oversized image every poll. Retry only after a new copy.
        self.sequence = sequence;
        self.previous = None;
    }
    pub fn copied(&mut self, key: CaptureKey) {
        self.previous = Some(key);
        // Re-read once: another program may replace our copy before the sequence is read.
        self.sequence = None;
    }
}

/// 剪贴板类型约定（nspasteboard.org）：密码管理器等用这些类型声明「敏感 / 临时」内容。
pub const MAC_CONCEALED_TYPES: [&str; 3] = [
    "org.nspasteboard.ConcealedType",
    "org.nspasteboard.TransientType",
    "com.agilebits.onepassword",
];
/// Chromium 系浏览器复制网页内容时附带的来源页面类型。
pub const MAC_SOURCE_URL_TYPE: &str = "org.chromium.source-url";
/// 复制方声明的来源应用 bundle ID。
pub const MAC_SOURCE_APP_TYPE: &str = "org.nspasteboard.source";

pub fn is_concealed_type(name: &str) -> bool {
    MAC_CONCEALED_TYPES.contains(&name)
}

/// 从 Windows「HTML Format」头部读取 `SourceURL:`；只看头部，不解析网页正文。
pub fn html_source_url(header: &str) -> Option<String> {
    header
        .lines()
        .take_while(|line| !line.trim_start().starts_with('<'))
        .find_map(|line| line.strip_prefix("SourceURL:"))
        .map(|url| url.trim().to_owned())
        .filter(|url| !url.is_empty())
}

/// Windows 只能拿到进程文件名，常见应用换成易读名称，其余保留文件名。
pub fn windows_app_name(executable: &str) -> Option<String> {
    let file = executable.rsplit(['\\', '/']).next()?.trim();
    let stem = file
        .strip_suffix(".exe")
        .or_else(|| file.strip_suffix(".EXE"))
        .unwrap_or(file);
    if stem.is_empty() {
        return None;
    }
    let known = match stem.to_ascii_lowercase().as_str() {
        "chrome" => "Chrome",
        "msedge" => "Edge",
        "firefox" => "Firefox",
        "brave" => "Brave",
        "opera" => "Opera",
        "code" => "VS Code",
        "cursor" => "Cursor",
        "explorer" => "文件资源管理器",
        "notepad" => "记事本",
        "winword" => "Word",
        "excel" => "Excel",
        "powerpnt" => "PowerPoint",
        "outlook" => "Outlook",
        "onenote" => "OneNote",
        "wechat" | "weixin" => "微信",
        "qq" => "QQ",
        "dingtalk" => "钉钉",
        "feishu" | "lark" => "飞书",
        "windowsterminal" => "终端",
        "cmd" => "命令提示符",
        "powershell" | "pwsh" => "PowerShell",
        "slack" => "Slack",
        "telegram" => "Telegram",
        "obsidian" => "Obsidian",
        "notion" => "Notion",
        _ => return Some(stem.to_owned()),
    };
    Some(known.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_helpers_read_html_header_and_app_names() {
        let header = "Version:0.9\r\nStartHTML:00000097\r\nSourceURL:https://linux.do/t/1?x=2\r\n<html><body>SourceURL:https://evil.test</body></html>";
        assert_eq!(
            html_source_url(header).as_deref(),
            Some("https://linux.do/t/1?x=2")
        );
        assert_eq!(
            html_source_url("<html>SourceURL:https://evil.test</html>"),
            None
        );
        assert_eq!(html_source_url("Version:0.9\r\nSourceURL:\r\n"), None);
        assert_eq!(
            windows_app_name("C:\\Program Files\\Google\\chrome.exe").as_deref(),
            Some("Chrome")
        );
        assert_eq!(
            windows_app_name("D:\\Tools\\原创工具.exe").as_deref(),
            Some("原创工具")
        );
        assert_eq!(windows_app_name(""), None);
        assert!(is_concealed_type("org.nspasteboard.ConcealedType"));
        assert!(!is_concealed_type("public.utf8-plain-text"));
    }
    #[test]
    fn sequence_captures_current_item_once_and_retries_failed_reads() {
        let mut state = CaptureState::default();
        let image = Some(CaptureKey::Image("test-image".into()));
        assert!(state.should_read(Some(14)));
        assert!(
            state.should_read(Some(14)),
            "failed reads must remain retryable"
        );
        assert!(state.needs_capture(&image));
        state.accept(Some(14), image.clone());
        assert!(!state.should_read(Some(14)));
        assert!(!state.needs_capture(&image));
        assert!(state.should_read(Some(15)));
        assert!(!state.needs_capture(&image));
        assert!(
            state.should_read(Some(0)),
            "sequence wrap is still a change"
        );
        assert!(!state.needs_capture(&image));
        state.accept(Some(0), None);
        assert!(
            state.needs_capture(&image),
            "image after clearing is a new copy"
        );
        state.copied(image.unwrap());
        assert!(state.should_read(Some(3)));
        assert!(!state.needs_capture(&Some(CaptureKey::Image("test-image".into()))));
        assert!(state.needs_capture(&Some(CaptureKey::Text("test-image".into()))));
        state.skip_invalid(Some(3));
        assert!(!state.should_read(Some(3)));
        assert!(state.should_read(Some(4)));
    }
    #[test]
    fn fallback_captures_current_item_once_and_empty_clipboards_are_not_records() {
        let mut state = CaptureState::default();
        let text = Some(CaptureKey::Text("original fixture".into()));
        assert!(state.should_read(None));
        assert!(state.needs_capture(&text));
        state.accept(None, text.clone());
        assert!(state.should_read(None));
        assert!(!state.needs_capture(&text));
        assert!(state.needs_capture(&Some(CaptureKey::Image("fixture".into()))));
        assert!(!state.needs_capture(&None));
        state.accept(None, None);
        assert!(!state.needs_capture(&None));
        assert!(state.needs_capture(&text));
    }
    #[test]
    fn resume_reset_captures_current_content_for_both_sequence_and_fallback() {
        let text = Some(CaptureKey::Text("resume fixture".into()));
        for sequence in [Some(5), None] {
            let mut state = CaptureState::default();
            state.accept(sequence, text.clone());
            assert!(!state.needs_capture(&text));
            // save_settings resets the state whenever paused changes.
            state = CaptureState::default();
            assert!(state.should_read(sequence));
            assert!(state.needs_capture(&text));
            state.accept(sequence, text.clone());
            assert!(!state.needs_capture(&text));
        }
    }
    #[test]
    fn initial_invalid_item_and_own_copy_do_not_repeat() {
        let mut state = CaptureState::default();
        assert!(state.should_read(Some(5)));
        state.skip_invalid(Some(5));
        assert!(!state.should_read(Some(5)));
        assert!(state.should_read(Some(6)));
        let text = CaptureKey::Text("own copy fixture".into());
        state = CaptureState::default();
        state.copied(text.clone());
        assert!(state.should_read(Some(7)));
        assert!(!state.needs_capture(&Some(text)));
        assert!(state.needs_capture(&Some(CaptureKey::Text("external fixture".into()))));
    }
}
