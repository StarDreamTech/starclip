//! Clipboard transition bookkeeping, tested without touching the system clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureKey {
    Text(String),
    Image(String),
    /// Joined native paths. Kept separate from `Text` so a file list and text
    /// that happens to read like a path never suppress each other.
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

#[cfg(test)]
mod tests {
    use super::*;
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
    fn file_lists_have_their_own_identity_space() {
        let files = CaptureKey::Files("/original/one.txt\n/original/two".into());
        let mut state = CaptureState::default();
        state.accept(Some(9), Some(files.clone()));
        assert!(!state.needs_capture(&Some(files.clone())));
        // The same joined paths as plain text must still be a new copy.
        assert!(state.needs_capture(&Some(CaptureKey::Text(
            "/original/one.txt\n/original/two".into()
        ))));
        // Reordering is a different list, so it is captured again.
        assert!(state.needs_capture(&Some(CaptureKey::Files(
            "/original/two\n/original/one.txt".into()
        ))));
        state.copied(files.clone());
        assert!(state.should_read(Some(10)));
        assert!(!state.needs_capture(&Some(files)));
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
