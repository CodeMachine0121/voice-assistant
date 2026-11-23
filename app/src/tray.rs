//! Menu-bar icon: what it looks like for each status, which menu entry does what,
//! and the glue to the system tray.
use crate::wiring::RepaintHandle;
use crossbeam_channel::Sender;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use voice_core::Status;

const ICON_SIZE: u32 = 32;
const APP_NAME: &str = "Voice Assistant";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayAppearance {
    pub tooltip: String,
    /// Short text next to the icon; empty = none.
    pub title: String,
    pub rgb: [u8; 3],
}

pub fn tray_appearance(status: &Status) -> TrayAppearance {
    let (state, title, rgb): (String, &str, [u8; 3]) = match status {
        Status::Idle => ("idle".into(), "", [140, 140, 146]),
        Status::Listening { action, .. } => (
            format!("listening ({})", action.label()),
            "Listening",
            [235, 64, 52],
        ),
        Status::Working(action) => (
            format!("working ({})", action.label()),
            "Working",
            [245, 166, 35],
        ),
        Status::Done => ("done".into(), "Done", [52, 199, 89]),
        Status::Cancelled => ("cancelled".into(), "Cancelled", [140, 140, 146]),
        Status::Problem(problem) => (problem.to_string(), "Problem", [255, 59, 48]),
    };
    TrayAppearance {
        tooltip: format!("{APP_NAME} – {state}"),
        title: title.to_string(),
        rgb,
    }
}

/// RGBA pixels of a filled, anti-aliased circle on a transparent square.
pub fn circle_rgba(size: u32, rgb: [u8; 3]) -> Vec<u8> {
    let centre = size as f32 / 2.0;
    let radius = centre - 2.0;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - centre;
            let dy = y as f32 + 0.5 - centre;
            let coverage = (radius + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
            pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], (coverage * 255.0) as u8]);
        }
    }
    pixels
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Settings,
    History,
    Quit,
}

/// Maps menu entries to what they mean.
#[derive(Debug, Clone)]
pub struct TrayMenuIds {
    pub settings: MenuId,
    pub history: MenuId,
    pub quit: MenuId,
}

impl TrayMenuIds {
    pub fn command_for(&self, id: &MenuId) -> Option<TrayCommand> {
        if *id == self.settings {
            Some(TrayCommand::Settings)
        } else if *id == self.history {
            Some(TrayCommand::History)
        } else if *id == self.quit {
            Some(TrayCommand::Quit)
        } else {
            None
        }
    }
}

/// The live menu-bar icon. Must be created on the main thread once the event loop runs.
pub struct Tray {
    icon: TrayIcon,
    shown: Option<TrayAppearance>,
}

impl Tray {
    /// Menu clicks arrive on `commands`; each one also wakes the UI.
    pub fn new(commands: Sender<TrayCommand>, repaint: RepaintHandle) -> Result<Self, String> {
        let settings = MenuItem::new("Settings…", true, None);
        let history = MenuItem::new("History…", true, None);
        let quit = MenuItem::new("Quit", true, None);
        let ids = TrayMenuIds {
            settings: settings.id().clone(),
            history: history.id().clone(),
            quit: quit.id().clone(),
        };
        let menu = Menu::new();
        menu.append_items(&[&settings, &history, &PredefinedMenuItem::separator(), &quit])
            .map_err(|e| e.to_string())?;
        MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
            if let Some(command) = ids.command_for(&event.id) {
                let _ = commands.send(command);
                repaint.request();
            }
        }));
        let idle = tray_appearance(&Status::Idle);
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(&idle.tooltip)
            .with_icon(icon_for(&idle)?)
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            icon,
            shown: Some(idle),
        })
    }

    /// Updates tooltip, title and icon when `appearance` differs from what is shown.
    pub fn show(&mut self, appearance: TrayAppearance) {
        if self.shown.as_ref() == Some(&appearance) {
            return;
        }
        if let Err(e) = self.apply(&appearance) {
            log::warn!("could not update the menu-bar icon: {e}");
        }
        self.shown = Some(appearance);
    }

    fn apply(&self, appearance: &TrayAppearance) -> Result<(), String> {
        self.icon
            .set_tooltip(Some(&appearance.tooltip))
            .map_err(|e| e.to_string())?;
        self.icon
            .set_icon(Some(icon_for(appearance)?))
            .map_err(|e| e.to_string())?;
        self.icon
            .set_title((!appearance.title.is_empty()).then_some(&appearance.title));
        Ok(())
    }
}

fn icon_for(appearance: &TrayAppearance) -> Result<Icon, String> {
    Icon::from_rgba(circle_rgba(ICON_SIZE, appearance.rgb), ICON_SIZE, ICON_SIZE)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use voice_core::{Action, Problem};

    fn listening(level: f32) -> Status {
        Status::Listening {
            action: Action::Dictate,
            level,
        }
    }

    #[test]
    fn tooltip_names_the_status() {
        let tip = |s: Status| tray_appearance(&s).tooltip;
        assert_eq!(tip(Status::Idle), "Voice Assistant – idle");
        assert_eq!(tip(listening(0.9)), "Voice Assistant – listening (Dictate)");
        assert_eq!(
            tip(Status::Working(Action::DictateInEnglish)),
            "Voice Assistant – working (English)"
        );
        assert_eq!(tip(Status::Done), "Voice Assistant – done");
        assert_eq!(tip(Status::Cancelled), "Voice Assistant – cancelled");
    }

    #[test]
    fn problem_tooltip_carries_the_message() {
        let a = tray_appearance(&Status::Problem(Problem::NoSpeech));
        assert_eq!(a.tooltip, "Voice Assistant – No speech heard");
        assert_eq!(a.title, "Problem");
    }

    #[test]
    fn idle_shows_no_title_and_others_do() {
        assert_eq!(tray_appearance(&Status::Idle).title, "");
        assert_eq!(tray_appearance(&listening(0.0)).title, "Listening");
        assert_eq!(
            tray_appearance(&Status::Working(Action::EditSelection)).title,
            "Working"
        );
        assert_eq!(tray_appearance(&Status::Done).title, "Done");
        assert_eq!(tray_appearance(&Status::Cancelled).title, "Cancelled");
    }

    #[test]
    fn colors_tell_statuses_apart() {
        let rgb = |s: Status| tray_appearance(&s).rgb;
        let working = || Status::Working(Action::Dictate);
        assert_ne!(rgb(Status::Idle), rgb(listening(0.0)));
        assert_ne!(rgb(listening(0.0)), rgb(working()));
        assert_ne!(rgb(working()), rgb(Status::Done));
        assert_ne!(rgb(Status::Done), rgb(Status::Problem(Problem::NoSpeech)));
        assert_eq!(rgb(Status::Idle), rgb(Status::Cancelled));
    }

    #[test]
    fn level_does_not_change_the_appearance() {
        assert_eq!(
            tray_appearance(&listening(0.1)),
            tray_appearance(&listening(0.9))
        );
    }

    #[test]
    fn circle_is_opaque_in_the_middle_and_clear_in_the_corner() {
        let size = 32u32;
        let px = circle_rgba(size, [10, 20, 30]);
        assert_eq!(px.len(), (size * size * 4) as usize);
        let at = |x: u32, y: u32| &px[((y * size + x) * 4) as usize..][..4];
        assert_eq!(at(16, 16), [10, 20, 30, 255]);
        assert_eq!(at(0, 0)[3], 0);
        let edge = at(2, 16)[3];
        assert!(edge > 0 && edge < 255, "edge pixel is anti-aliased: {edge}");
    }

    #[test]
    fn menu_entries_map_to_commands() {
        let ids = TrayMenuIds {
            settings: MenuId::new("s"),
            history: MenuId::new("h"),
            quit: MenuId::new("q"),
        };
        assert_eq!(
            ids.command_for(&MenuId::new("s")),
            Some(TrayCommand::Settings)
        );
        assert_eq!(
            ids.command_for(&MenuId::new("h")),
            Some(TrayCommand::History)
        );
        assert_eq!(ids.command_for(&MenuId::new("q")), Some(TrayCommand::Quit));
        assert_eq!(ids.command_for(&MenuId::new("other")), None);
    }
}
