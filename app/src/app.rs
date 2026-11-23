//! The eframe app: owns the windows and the menu-bar icon, and moves messages
//! between the engine, the tray and the view-model. Drawing and OS glue only.
use crate::model::AppModel;
use crate::tray::{tray_appearance, Tray, TrayCommand};
use crate::ui;
use crate::wiring::Services;
use crossbeam_channel::{unbounded, Receiver};
use eframe::egui::{self, ViewportCommand};
use std::time::{Duration, Instant};

pub struct VoiceApp {
    services: Services,
    model: AppModel,
    tray: Option<Tray>,
    commands: Receiver<TrayCommand>,
}

impl VoiceApp {
    pub fn new(cc: &eframe::CreationContext<'_>, services: Services) -> Self {
        services.repaint.connect(cc.egui_ctx.clone());

        let (tx, commands) = unbounded();
        let tray = match Tray::new(tx, services.repaint.clone()) {
            Ok(tray) => Some(tray),
            Err(e) => {
                log::error!("no menu-bar icon: {e}");
                None
            }
        };
        let now = Instant::now();
        let mut model = AppModel::new(now);
        if tray.is_none() {
            // Without the menu there is no other way in; open Settings right away.
            model.handle_command(TrayCommand::Settings, &services, now);
        }
        Self {
            services,
            model,
            tray,
            commands,
        }
    }
}

impl eframe::App for VoiceApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let now = Instant::now();
        take_in_messages(ctx, &mut self.model, &self.services, &self.commands, now);
        // The menu-bar icon is not an egui window, so it follows the status here,
        // where time passing is noticed even while `ui` is skipped.
        if let Some(tray) = &mut self.tray {
            tray.show(tray_appearance(&self.model.indicator.effective_status(now)));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        if self.model.quit_requested {
            return;
        }

        if let Some(view) = self.model.indicator.view(now) {
            ui::indicator::show(&ctx, &view);
        }

        if let Some(session) = &mut self.model.settings {
            let focus = &mut self.model.settings_focus;
            if ui::settings::show(&ctx, session, &self.services, focus, now) {
                self.model.close_settings(&self.services);
            }
        }
        if self.model.history_open
            && ui::history::show(
                &ctx,
                &self.model.history,
                &mut self.model.history_focus,
                now,
            )
        {
            self.model.close_history(&self.services);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }
}

/// Takes in the engine's statuses and the menu commands. Runs from `App::logic`,
/// which eframe calls on every wake-up, also while the (invisible) root window
/// counts as hidden and `ui` is skipped; macOS marks it hidden e.g. while the screen
/// is locked. Whatever arrived then still has to reach the screen, so a full frame
/// is asked for. Wakes up again when the shown status expires, so the menu-bar
/// icon and the indicator go back to idle without waiting for another event.
fn take_in_messages(
    ctx: &egui::Context,
    model: &mut AppModel,
    services: &Services,
    commands: &Receiver<TrayCommand>,
    now: Instant,
) {
    let mut changed = model.drain_statuses(services, now);
    for command in commands.try_iter() {
        changed = true;
        model.handle_command(command, services, now);
    }
    if model.quit_requested {
        ctx.send_viewport_cmd(ViewportCommand::Close);
        return;
    }
    let expired = model.status_expired(now);
    let hidden = ctx.input(|i| i.viewport().visible() == Some(false));
    if (changed || expired) && hidden {
        ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
        ctx.request_repaint();
    }
    if let Some(wait) = model.indicator.time_to_expiry(now) {
        ctx.request_repaint_after(wait + Duration::from_millis(20));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wiring::testing::{test_services, FocusMove};
    use eframe::egui::{RawInput, ViewportId, ViewportInfo};

    /// Runs only the app logic, as eframe does while the root window counts as
    /// hidden (`occluded`), and returns the root window's commands.
    fn logic_only(
        occluded: bool,
        model: &mut AppModel,
        services: &Services,
        commands: &Receiver<TrayCommand>,
    ) -> (egui::Context, Vec<ViewportCommand>) {
        logic_at(Instant::now(), occluded, model, services, commands)
    }

    fn logic_at(
        now: Instant,
        occluded: bool,
        model: &mut AppModel,
        services: &Services,
        commands: &Receiver<TrayCommand>,
    ) -> (egui::Context, Vec<ViewportCommand>) {
        let ctx = egui::Context::default();
        let mut input = RawInput::default();
        input.viewports.insert(
            ViewportId::ROOT,
            ViewportInfo {
                minimized: Some(false),
                occluded: Some(occluded),
                ..ViewportInfo::default()
            },
        );
        let output = ctx.run_logic(&input, |ctx| {
            take_in_messages(ctx, model, services, commands, now)
        });
        let root = output
            .viewport_commands
            .get(&ViewportId::ROOT)
            .cloned()
            .unwrap_or_default();
        (ctx, root)
    }

    #[test]
    fn a_menu_command_while_hidden_brings_the_app_up_and_asks_for_a_painted_frame() {
        let (services, probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        let (tx, rx) = unbounded();
        tx.send(TrayCommand::Settings).unwrap();

        let (ctx, commands) = logic_only(true, &mut model, &services, &rx);

        assert!(model.settings.is_some());
        assert_eq!(probe.focus_moves(), [FocusMove::BringToFront]);
        assert!(commands.contains(&ViewportCommand::RequestPaintWhileHidden));
        assert!(ctx.has_requested_repaint());
    }

    #[test]
    fn a_status_while_hidden_asks_for_a_painted_frame() {
        let (services, probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        let (_tx, rx) = unbounded();
        probe.send_status(voice_core::Status::Working(voice_core::Action::Dictate));

        let (_ctx, commands) = logic_only(true, &mut model, &services, &rx);

        assert!(commands.contains(&ViewportCommand::RequestPaintWhileHidden));
    }

    #[test]
    fn nothing_new_while_hidden_asks_for_nothing() {
        let (services, _probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        let (_tx, rx) = unbounded();

        let (_ctx, commands) = logic_only(true, &mut model, &services, &rx);

        assert!(commands.is_empty());
    }

    #[test]
    fn while_shown_the_regular_frame_follows_without_an_extra_request() {
        let (services, _probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        let (tx, rx) = unbounded();
        tx.send(TrayCommand::History).unwrap();

        let (_ctx, commands) = logic_only(false, &mut model, &services, &rx);

        assert!(model.history_open);
        assert!(!commands.contains(&ViewportCommand::RequestPaintWhileHidden));
    }

    #[test]
    fn a_message_expiring_while_hidden_asks_for_a_painted_frame() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        let (_tx, rx) = unbounded();
        probe.send_status(voice_core::Status::Done);
        logic_at(now, true, &mut model, &services, &rx);
        let expiry = now + model.indicator.time_to_expiry(now).unwrap();

        let (_ctx, commands) = logic_at(expiry, true, &mut model, &services, &rx);

        assert!(commands.contains(&ViewportCommand::RequestPaintWhileHidden));
    }

    #[test]
    fn logic_wakes_up_again_when_the_shown_message_expires() {
        let (services, probe) = test_services();
        let now = Instant::now();
        let mut model = AppModel::new(now);
        let (_tx, rx) = unbounded();
        probe.send_status(voice_core::Status::Done);
        let ctx = egui::Context::default();
        // A new context starts with an immediate repaint pending; serve it first.
        let _ = ctx.run_logic(&RawInput::default(), |_| {});
        let delays = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = delays.clone();
        ctx.set_request_repaint_callback(move |info| seen.lock().unwrap().push(info.delay));

        let _ = ctx.run_logic(&RawInput::default(), |ctx| {
            take_in_messages(ctx, &mut model, &services, &rx, now)
        });

        let wait = model.indicator.time_to_expiry(now).unwrap();
        assert!(delays
            .lock()
            .unwrap()
            .iter()
            .any(|d| *d >= wait && *d <= wait + Duration::from_millis(20)));
    }

    #[test]
    fn quitting_closes_the_app() {
        let (services, _probe) = test_services();
        let mut model = AppModel::new(Instant::now());
        let (tx, rx) = unbounded();
        tx.send(TrayCommand::Quit).unwrap();

        let (_ctx, commands) = logic_only(true, &mut model, &services, &rx);

        assert_eq!(commands, [ViewportCommand::Close]);
    }
}
