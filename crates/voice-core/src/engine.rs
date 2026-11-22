//! Background engine: owns the controller and recorder, runs the pipeline,
//! and publishes `Status` updates. The UI only sends shortcut events and settings.
use crate::{DictationPipeline, Recorder, Settings, ShortcutEvent, Status};
use crossbeam_channel::{Receiver, Sender};

pub struct EngineDeps {
    pub recorder: Box<dyn Recorder>,
    pub pipeline: DictationPipeline,
    pub settings: Settings,
}

/// Handle used by the UI and the shortcut listener.
#[derive(Clone)]
pub struct EngineHandle {
    pub shortcuts: Sender<ShortcutEvent>,
    pub settings: Sender<Settings>,
    pub status: Receiver<Status>,
}

pub struct Engine;

impl Engine {
    /// Spawns the engine thread. `on_status` is called after every status change
    /// (e.g. to request a UI repaint).
    pub fn spawn(deps: EngineDeps, on_status: Box<dyn Fn() + Send>) -> EngineHandle {
        todo!()
    }
}
