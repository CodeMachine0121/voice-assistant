//! Voice Assistant: menu-bar dictation app. `wiring` is the only module that knows
//! the concrete adapters.
mod app;
mod focus;
mod history_view;
mod indicator;
mod model;
mod permissions_view;
mod settings_form;
mod shortcut_names;
mod tasks;
mod tray;
mod ui;
mod wiring;

use eframe::egui::ViewportBuilder;

/// No Dock icon and no menu bar of its own: the app lives in the menu bar.
#[cfg(target_os = "macos")]
fn accessory_app(builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>) {
    use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
    builder.with_activation_policy(ActivationPolicy::Accessory);
}

fn main() -> eframe::Result {
    env_logger::init();
    let services = wiring::build_services();
    let options = eframe::NativeOptions {
        viewport: ViewportBuilder::default()
            .with_title("Voice Assistant")
            .with_visible(false)
            .with_taskbar(false)
            .with_inner_size([1.0, 1.0]),
        #[cfg(target_os = "macos")]
        event_loop_builder: Some(Box::new(accessory_app)),
        ..Default::default()
    };
    eframe::run_native(
        "Voice Assistant",
        options,
        Box::new(|cc| Ok(Box::new(app::VoiceApp::new(cc, services)))),
    )
}
