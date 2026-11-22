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
        // The root window is hidden; keep running frames so the app reacts to the
        // engine and the tray without any visible window.
        cc.egui_ctx
            .send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);

        let (tx, commands) = unbounded();
        let tray = match Tray::new(tx, services.repaint.clone()) {
            Ok(tray) => Some(tray),
            Err(e) => {
                log::error!("no menu-bar icon: {e}");
                None
            }
        };
        let mut model = AppModel::new(Instant::now());
        if tray.is_none() {
            // Without the menu there is no other way in; open Settings right away.
            model.handle_command(TrayCommand::Settings, &services);
        }
        Self {
            services,
            model,
            tray,
            commands,
        }
    }

    fn focus(ctx: &egui::Context, command: TrayCommand) {
        match command {
            TrayCommand::Settings => ui::settings::focus(ctx),
            TrayCommand::History => ui::history::focus(ctx),
            TrayCommand::Quit => {}
        }
    }
}

impl eframe::App for VoiceApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();

        self.model.drain_statuses(&self.services, now);
        let commands: Vec<_> = self.commands.try_iter().collect();
        for command in commands {
            self.model.handle_command(command, &self.services);
            Self::focus(&ctx, command);
        }
        if self.model.quit_requested {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        if let Some(tray) = &mut self.tray {
            tray.show(tray_appearance(&self.model.indicator.effective_status(now)));
        }
        if let Some(view) = self.model.indicator.view(now) {
            ui::indicator::show(&ctx, &view);
        }
        if let Some(wait) = self.model.indicator.time_to_expiry(now) {
            ctx.request_repaint_after(wait + Duration::from_millis(20));
        }

        if let Some(session) = &mut self.model.settings {
            if ui::settings::show(&ctx, session, &self.services) {
                self.model.close_settings();
            }
        }
        if self.model.history_open && ui::history::show(&ctx, &self.model.history) {
            self.model.close_history();
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }
}
