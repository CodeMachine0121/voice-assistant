//! Slow things the Settings window starts (listing the writing assistant models) run on
//! their own thread and report back over a channel.
use crate::wiring::{ModelListFn, RepaintHandle};
use crossbeam_channel::Sender;
use std::thread::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEvent {
    Models(Result<Vec<String>, String>),
}

pub fn spawn_list_models(
    list: ModelListFn,
    url: String,
    events: Sender<TaskEvent>,
    repaint: RepaintHandle,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let _ = events.send(TaskEvent::Models(list(&url)));
        repaint.request();
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ModelsState {
    #[default]
    NotLoaded,
    Loading,
    Loaded(Vec<String>),
    Failed(String),
}

/// Shown in a model list while the local AI app is being asked.
pub const CHECKING: &str = "Checking…";
/// Shown in a model list when the local AI app reports nothing to pick.
pub const NO_MODELS: &str = "No models — open your local AI app and load one";

impl ModelsState {
    /// Result of "Check connection": what the local AI app at `address` answered.
    pub fn connection_line(&self, address: &str) -> Option<String> {
        match self {
            ModelsState::NotLoaded => None,
            ModelsState::Loading => Some(CHECKING.to_string()),
            ModelsState::Loaded(models) => Some(match models.len() {
                0 => "Connected — no models available".to_string(),
                1 => "Connected — 1 model available".to_string(),
                n => format!("Connected — {n} models available"),
            }),
            ModelsState::Failed(_) => Some(format!("Local AI app not reachable at {address}")),
        }
    }

    /// Note under a model list while it has nothing to pick.
    pub fn list_note(&self) -> Option<&'static str> {
        match self {
            ModelsState::Loading => Some(CHECKING),
            ModelsState::Loaded(models) if models.is_empty() => Some(NO_MODELS),
            ModelsState::Failed(_) => Some(NO_MODELS),
            _ => None,
        }
    }

    pub fn models(&self) -> &[String] {
        match self {
            ModelsState::Loaded(models) => models,
            _ => &[],
        }
    }
}

/// What the Settings window knows about its background work.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskState {
    pub models: ModelsState,
}

impl TaskState {
    pub fn apply(&mut self, event: TaskEvent) {
        match event {
            TaskEvent::Models(Ok(models)) => self.models = ModelsState::Loaded(models),
            TaskEvent::Models(Err(e)) => self.models = ModelsState::Failed(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use std::sync::Arc;

    #[test]
    fn listing_models_reports_the_result() {
        let (tx, rx) = unbounded();
        let list: ModelListFn = Arc::new(|url| Ok(vec![format!("m@{url}")]));
        spawn_list_models(list, "http://x".into(), tx, RepaintHandle::default())
            .join()
            .unwrap();
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![TaskEvent::Models(Ok(vec!["m@http://x".to_string()]))]
        );
    }

    #[test]
    fn listing_models_reports_a_failure() {
        let (tx, rx) = unbounded();
        let list: ModelListFn = Arc::new(|_| Err("refused".to_string()));
        spawn_list_models(list, String::new(), tx, RepaintHandle::default())
            .join()
            .unwrap();
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![TaskEvent::Models(Err("refused".to_string()))]
        );
    }

    #[test]
    fn a_dropped_receiver_does_not_crash_the_workers() {
        let (tx, rx) = unbounded();
        drop(rx);
        let list: ModelListFn = Arc::new(|_| Ok(vec![]));
        spawn_list_models(list, String::new(), tx, RepaintHandle::default())
            .join()
            .unwrap();
    }

    #[test]
    fn connection_line_reports_what_the_local_ai_app_answered() {
        let line = |s: &ModelsState| s.connection_line("http://localhost:1234/v1");
        assert_eq!(line(&ModelsState::NotLoaded), None);
        assert_eq!(line(&ModelsState::Loading).as_deref(), Some("Checking…"));
        assert_eq!(
            line(&ModelsState::Loaded(vec!["a".into(), "b".into()])).as_deref(),
            Some("Connected — 2 models available")
        );
        assert_eq!(
            line(&ModelsState::Loaded(vec!["a".into()])).as_deref(),
            Some("Connected — 1 model available")
        );
        assert_eq!(
            line(&ModelsState::Loaded(vec![])).as_deref(),
            Some("Connected — no models available")
        );
        assert_eq!(
            line(&ModelsState::Failed("refused".into())).as_deref(),
            Some("Local AI app not reachable at http://localhost:1234/v1")
        );
    }

    #[test]
    fn list_note_explains_an_empty_list() {
        assert_eq!(ModelsState::NotLoaded.list_note(), None);
        assert_eq!(ModelsState::Loading.list_note(), Some("Checking…"));
        assert_eq!(
            ModelsState::Loaded(vec![]).list_note(),
            Some("No models — open your local AI app and load one")
        );
        assert_eq!(
            ModelsState::Failed("down".into()).list_note(),
            Some("No models — open your local AI app and load one")
        );
        assert_eq!(ModelsState::Loaded(vec!["a".into()]).list_note(), None);
    }

    #[test]
    fn models_are_only_listed_when_loaded() {
        assert!(ModelsState::Loading.models().is_empty());
        assert_eq!(ModelsState::Loaded(vec!["a".into()]).models(), ["a"]);
    }

    #[test]
    fn events_move_the_state() {
        let mut state = TaskState::default();
        state.apply(TaskEvent::Models(Ok(vec!["a".into()])));
        assert_eq!(state.models, ModelsState::Loaded(vec!["a".into()]));
        state.apply(TaskEvent::Models(Err("e".into())));
        assert_eq!(state.models, ModelsState::Failed("e".into()));
    }
}
