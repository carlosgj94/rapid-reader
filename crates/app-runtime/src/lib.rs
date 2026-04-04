#![no_std]
#![allow(dead_code)]

use domain::{runtime::Command, selectors::select_active_screen, store::Store};

pub mod animation;
pub mod components;
pub mod navigation;
pub mod screens;
pub mod view_models;

pub use animation::{AnimationDescriptor, MotionDirection, TransitionPlan};
pub use components::{
    ComponentId, ContentListShell, DashboardShell, ParagraphNavigationShell, PreparedScreen,
    ReaderShell, SettingsShell,
};
pub use navigation::NavigationState;
pub use screens::Screen;
pub use view_models::{
    ActiveScreenModel, ContentListScreenModel, DashboardScreenModel, ParagraphNavigationModel,
    ReaderScreenModel, SettingsScreenModel,
};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct ScreenUpdate {
    pub screen: Screen,
    pub prepared: PreparedScreen,
    pub transition: TransitionPlan,
}

#[derive(Debug, Default)]
pub struct AppRuntime {
    navigation: NavigationState,
    previous: Option<ScreenUpdate>,
}

impl AppRuntime {
    pub const fn new() -> Self {
        Self {
            navigation: NavigationState::new(),
            previous: None,
        }
    }

    pub fn tick(&mut self, store: &Store) -> ScreenUpdate {
        let active = select_active_screen(store);
        let (screen, prepared) = components::compose(active);
        let transition = plan_transition(self.previous, screen, prepared);
        let update = ScreenUpdate {
            screen,
            prepared,
            transition,
        };

        self.previous = Some(update);
        update
    }

    pub fn handle_input_gesture(&mut self, gesture: domain::input::InputGesture) -> Command {
        let _ = self.navigation;
        NavigationState::command_for_gesture(gesture)
    }
}

fn plan_transition(
    previous: Option<ScreenUpdate>,
    screen: Screen,
    prepared: PreparedScreen,
) -> TransitionPlan {
    let Some(previous) = previous else {
        return TransitionPlan::none();
    };

    use animation::{AnimationDescriptor as A, MotionDirection as D, TransitionPlan as T};

    match (previous.screen, screen, previous.prepared, prepared) {
        (
            Screen::Dashboard,
            Screen::Dashboard,
            PreparedScreen::Dashboard(old),
            PreparedScreen::Dashboard(new),
        ) if old.items[1].label != new.items[1].label => {
            T::new(A::BandReveal(direction_for_dashboard(&old, &new)), 3, 70)
        }
        (
            old_screen,
            new_screen,
            PreparedScreen::Collection(old),
            PreparedScreen::Collection(new),
        ) if old_screen == new_screen && old.rows[1].title != new.rows[1].title => {
            T::new(A::ListStep(direction_for_rows(&old, &new)), 3, 70)
        }
        (old_screen, Screen::Reader, _, PreparedScreen::Reader(_))
            if is_collection_screen(old_screen) =>
        {
            T::new(A::ReaderEnter, 3, 80)
        }
        (Screen::Reader, new_screen, PreparedScreen::Reader(_), _)
            if is_collection_screen(new_screen) =>
        {
            T::new(A::ReaderExit, 3, 80)
        }
        (
            Screen::Reader,
            Screen::Reader,
            PreparedScreen::Reader(old),
            PreparedScreen::Reader(new),
        ) if old.pause_modal.is_none() && new.pause_modal.is_some() => {
            T::new(A::ModalReveal, 3, 80)
        }
        (
            Screen::Reader,
            Screen::Reader,
            PreparedScreen::Reader(old),
            PreparedScreen::Reader(new),
        ) if old.pause_modal.is_some() && new.pause_modal.is_none() => T::new(A::ModalHide, 3, 80),
        (
            Screen::ParagraphNavigation,
            Screen::ParagraphNavigation,
            PreparedScreen::ParagraphNavigation(old),
            PreparedScreen::ParagraphNavigation(new),
        ) if old.rail.selected_index != new.rail.selected_index => T::new(
            A::ParagraphTickMove(if old.rail.selected_index < new.rail.selected_index {
                D::Forward
            } else {
                D::Backward
            }),
            3,
            70,
        ),
        (
            Screen::Settings,
            Screen::Settings,
            PreparedScreen::Settings(old),
            PreparedScreen::Settings(new),
        ) if old.mode == new.mode
            && old
                .rows
                .iter()
                .zip(new.rows.iter())
                .any(|(a, b)| a.selected != b.selected) =>
        {
            T::new(A::BandReveal(direction_for_settings(&old, &new)), 3, 70)
        }
        (
            Screen::Settings,
            Screen::Settings,
            PreparedScreen::Settings(old),
            PreparedScreen::Settings(new),
        ) if old.mode == domain::ui::SettingsMode::SpeedEdit
            && new.mode == domain::ui::SettingsMode::SpeedEdit
            && old.rows[0].value != new.rows[0].value =>
        {
            T::new(A::SettingsValuePulse, 3, 70)
        }
        (
            Screen::Settings,
            Screen::Settings,
            PreparedScreen::Settings(old),
            PreparedScreen::Settings(new),
        ) if old.mode == domain::ui::SettingsMode::AppearanceEdit
            && new.mode == domain::ui::SettingsMode::AppearanceEdit
            && old.rows[1].value != new.rows[1].value =>
        {
            T::new(A::AppearanceFlip, 3, 70)
        }
        (Screen::Settings, Screen::Settings, _, PreparedScreen::Settings(new))
            if matches!(new.mode, domain::ui::SettingsMode::RefreshLoading) =>
        {
            T::new(A::RefreshPulse, 4, 60)
        }
        _ => TransitionPlan::none(),
    }
}

fn direction_for_dashboard(
    old: &components::DashboardShell,
    new: &components::DashboardShell,
) -> MotionDirection {
    if old.items[0].label == new.items[1].label {
        MotionDirection::Backward
    } else {
        MotionDirection::Forward
    }
}

fn direction_for_rows(
    old: &components::ContentListShell,
    new: &components::ContentListShell,
) -> MotionDirection {
    if old.rows[0].title == new.rows[1].title {
        MotionDirection::Backward
    } else {
        MotionDirection::Forward
    }
}

fn direction_for_settings(
    old: &components::SettingsShell,
    new: &components::SettingsShell,
) -> MotionDirection {
    let old_index = old.rows.iter().position(|row| row.selected).unwrap_or(0);
    let new_index = new.rows.iter().position(|row| row.selected).unwrap_or(0);

    if new_index >= old_index {
        MotionDirection::Forward
    } else {
        MotionDirection::Backward
    }
}

const fn is_collection_screen(screen: Screen) -> bool {
    matches!(
        screen,
        Screen::Saved | Screen::Inbox | Screen::Recommendations
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{
        input::{InputGesture, RotationDirection},
        runtime::UiCommand,
        store::Store,
    };

    #[test]
    fn gesture_mapping_uses_typed_ui_commands() {
        let mut runtime = AppRuntime::new();
        let command = runtime.handle_input_gesture(InputGesture::Click);

        assert_eq!(command, Command::Ui(UiCommand::Confirm));
    }

    #[test]
    fn clockwise_rotation_maps_to_focus_next() {
        let mut runtime = AppRuntime::new();
        let command = runtime.handle_input_gesture(InputGesture::Rotate {
            direction: RotationDirection::Clockwise,
        });

        assert_eq!(command, Command::Ui(UiCommand::FocusNext));
    }

    #[test]
    fn counterclockwise_rotation_maps_to_focus_previous() {
        let mut runtime = AppRuntime::new();
        let command = runtime.handle_input_gesture(InputGesture::Rotate {
            direction: RotationDirection::CounterClockwise,
        });

        assert_eq!(command, Command::Ui(UiCommand::FocusPrevious));
    }

    #[test]
    fn first_tick_has_no_transition() {
        let mut runtime = AppRuntime::new();
        let store = Store::new();

        let update = runtime.tick(&store);

        assert_eq!(update.transition, TransitionPlan::none());
    }
}
