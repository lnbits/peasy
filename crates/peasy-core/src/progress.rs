//! Native progress carries fixed stages only, never subprocess text or AI data.
use crate::OperationStage;
use std::{cell::RefCell, sync::Arc};

pub type Sink = Arc<dyn Fn(OperationStage) + Send + Sync>;
thread_local! { static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) }; }
pub fn current() -> Option<Sink> {
    SINK.with(|s| s.borrow().clone())
}
pub fn report(stage: OperationStage) {
    if let Some(sink) = current() {
        sink(stage);
    }
}
pub fn scope<T>(sink: Option<Sink>, work: impl FnOnce() -> T) -> T {
    struct Restore(Option<Sink>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SINK.with(|s| *s.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(SINK.with(|s| s.replace(sink)));
    work()
}

pub fn nix_stage(line: &[u8]) -> Option<OperationStage> {
    let value: serde_json::Value = serde_json::from_slice(line.strip_prefix(b"@nix ")?).ok()?;
    if value["action"] != "start" {
        return None;
    }
    match value["type"].as_u64()? {
        100 | 101 | 108 => Some(OperationStage::Downloading),
        105 => Some(OperationStage::Building),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_known_nix_activities_become_progress() {
        assert_eq!(
            nix_stage(br#"@nix {"action":"start","type":108}"#),
            Some(OperationStage::Downloading)
        );
        assert_eq!(
            nix_stage(br#"@nix {"action":"start","type":105}"#),
            Some(OperationStage::Building)
        );
        assert_eq!(
            nix_stage(br#"@nix {"action":"msg","msg":"building a fake result"}"#),
            None
        );
        assert_eq!(nix_stage(b"hostile unstructured output"), None);
    }
}
