//! Slow things the Settings window starts (listing models, downloading the speech
//! model) run on their own thread and report back over a channel.
use crate::wiring::{ModelDownloadFn, ModelListFn, RepaintHandle};
use crossbeam_channel::Sender;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskEvent {
    Models(Result<Vec<String>, String>),
    DownloadProgress { done: u64, total: Option<u64> },
    DownloadFinished(Result<(), String>),
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

pub fn spawn_download(
    download: ModelDownloadFn,
    target: PathBuf,
    events: Sender<TaskEvent>,
    repaint: RepaintHandle,
) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let progress_events = events.clone();
        let progress_repaint = repaint.clone();
        let result = download(&target, &mut |done, total| {
            let _ = progress_events.send(TaskEvent::DownloadProgress { done, total });
            progress_repaint.request();
        });
        let _ = events.send(TaskEvent::DownloadFinished(result));
        repaint.request();
    })
}

/// Where "Download default model" puts the file: the typed location when there is
/// one, otherwise the default location.
pub fn download_target(typed: &str, default: &Path) -> PathBuf {
    let typed = typed.trim();
    if typed.is_empty() {
        default.to_path_buf()
    } else {
        PathBuf::from(typed)
    }
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

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DownloadState {
    #[default]
    Idle,
    Running {
        done: u64,
        total: Option<u64>,
    },
    Finished,
    Failed(String),
}

impl DownloadState {
    pub fn is_running(&self) -> bool {
        matches!(self, DownloadState::Running { .. })
    }

    /// Progress in [0.0, 1.0] when the total size is known.
    pub fn fraction(&self) -> Option<f32> {
        match self {
            DownloadState::Running {
                done,
                total: Some(total),
            } if *total > 0 => Some((*done as f64 / *total as f64).clamp(0.0, 1.0) as f32),
            DownloadState::Finished => Some(1.0),
            _ => None,
        }
    }

    pub fn message(&self) -> Option<String> {
        match self {
            DownloadState::Idle => None,
            DownloadState::Running {
                done,
                total: Some(total),
            } => Some(format!(
                "Downloading… {} of {}",
                format_bytes(*done),
                format_bytes(*total)
            )),
            DownloadState::Running { done, total: None } => {
                Some(format!("Downloading… {}", format_bytes(*done)))
            }
            DownloadState::Finished => Some("Speech model downloaded.".to_string()),
            DownloadState::Failed(e) => Some(format!("Download failed: {e}")),
        }
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else {
        format!("{:.0} KB", b / 1024.0)
    }
}

/// What the Settings window knows about its background work.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskState {
    pub models: ModelsState,
    pub download: DownloadState,
}

impl TaskState {
    pub fn apply(&mut self, event: TaskEvent) {
        match event {
            TaskEvent::Models(Ok(models)) => self.models = ModelsState::Loaded(models),
            TaskEvent::Models(Err(e)) => self.models = ModelsState::Failed(e),
            TaskEvent::DownloadProgress { done, total } => {
                self.download = DownloadState::Running { done, total }
            }
            TaskEvent::DownloadFinished(Ok(())) => self.download = DownloadState::Finished,
            TaskEvent::DownloadFinished(Err(e)) => self.download = DownloadState::Failed(e),
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
    fn download_reports_progress_then_the_result() {
        let (tx, rx) = unbounded();
        let download: ModelDownloadFn = Arc::new(|path, progress| {
            assert_eq!(path, Path::new("/m.bin"));
            progress(10, Some(100));
            progress(100, Some(100));
            Ok(())
        });
        spawn_download(download, "/m.bin".into(), tx, RepaintHandle::default())
            .join()
            .unwrap();
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![
                TaskEvent::DownloadProgress {
                    done: 10,
                    total: Some(100)
                },
                TaskEvent::DownloadProgress {
                    done: 100,
                    total: Some(100)
                },
                TaskEvent::DownloadFinished(Ok(())),
            ]
        );
    }

    #[test]
    fn download_reports_a_failure() {
        let (tx, rx) = unbounded();
        let download: ModelDownloadFn = Arc::new(|_, _| Err("offline".to_string()));
        spawn_download(download, "/m.bin".into(), tx, RepaintHandle::default())
            .join()
            .unwrap();
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![TaskEvent::DownloadFinished(Err("offline".to_string()))]
        );
    }

    #[test]
    fn a_dropped_receiver_does_not_crash_the_workers() {
        let (tx, rx) = unbounded();
        drop(rx);
        let list: ModelListFn = Arc::new(|_| Ok(vec![]));
        spawn_list_models(list, String::new(), tx.clone(), RepaintHandle::default())
            .join()
            .unwrap();
        let download: ModelDownloadFn = Arc::new(|_, progress| {
            progress(1, None);
            Ok(())
        });
        spawn_download(download, "/m".into(), tx, RepaintHandle::default())
            .join()
            .unwrap();
    }

    #[test]
    fn download_target_prefers_the_typed_path() {
        let default = Path::new("/default.bin");
        assert_eq!(download_target("  ", default), default);
        assert_eq!(
            download_target(" /mine.bin ", default),
            Path::new("/mine.bin")
        );
    }

    #[test]
    fn bytes_are_human_readable() {
        assert_eq!(format_bytes(512), "0 KB");
        assert_eq!(format_bytes(2048), "2 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 * 1024 / 2), "1.5 GB");
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
    fn download_fraction() {
        assert_eq!(DownloadState::Idle.fraction(), None);
        assert_eq!(
            DownloadState::Running {
                done: 25,
                total: Some(100)
            }
            .fraction(),
            Some(0.25)
        );
        assert_eq!(
            DownloadState::Running {
                done: 500,
                total: Some(100)
            }
            .fraction(),
            Some(1.0)
        );
        assert_eq!(
            DownloadState::Running {
                done: 5,
                total: Some(0)
            }
            .fraction(),
            None
        );
        assert_eq!(
            DownloadState::Running {
                done: 5,
                total: None
            }
            .fraction(),
            None
        );
        assert_eq!(DownloadState::Finished.fraction(), Some(1.0));
        assert_eq!(DownloadState::Failed("x".into()).fraction(), None);
    }

    #[test]
    fn download_messages() {
        assert_eq!(DownloadState::Idle.message(), None);
        assert_eq!(
            DownloadState::Running {
                done: 1024 * 1024,
                total: Some(10 * 1024 * 1024)
            }
            .message()
            .as_deref(),
            Some("Downloading… 1 MB of 10 MB")
        );
        assert_eq!(
            DownloadState::Running {
                done: 2048,
                total: None
            }
            .message()
            .as_deref(),
            Some("Downloading… 2 KB")
        );
        assert_eq!(
            DownloadState::Finished.message().as_deref(),
            Some("Speech model downloaded.")
        );
        assert_eq!(
            DownloadState::Failed("offline".into()).message().as_deref(),
            Some("Download failed: offline")
        );
    }

    #[test]
    fn only_a_running_download_is_running() {
        assert!(!DownloadState::Idle.is_running());
        assert!(DownloadState::Running {
            done: 0,
            total: None
        }
        .is_running());
        assert!(!DownloadState::Finished.is_running());
    }

    #[test]
    fn events_move_the_state() {
        let mut state = TaskState::default();
        state.apply(TaskEvent::Models(Ok(vec!["a".into()])));
        assert_eq!(state.models, ModelsState::Loaded(vec!["a".into()]));
        state.apply(TaskEvent::Models(Err("e".into())));
        assert_eq!(state.models, ModelsState::Failed("e".into()));
        state.apply(TaskEvent::DownloadProgress {
            done: 1,
            total: Some(2),
        });
        assert_eq!(
            state.download,
            DownloadState::Running {
                done: 1,
                total: Some(2)
            }
        );
        state.apply(TaskEvent::DownloadFinished(Ok(())));
        assert_eq!(state.download, DownloadState::Finished);
        state.apply(TaskEvent::DownloadFinished(Err("x".into())));
        assert_eq!(state.download, DownloadState::Failed("x".into()));
    }
}
