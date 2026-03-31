#![no_std]
#![allow(dead_code)]

use domain::input::InputGesture;

pub mod animation;
pub mod components;
pub mod navigation;
pub mod screens;
pub mod view_models;

pub use animation::AnimationDescriptor;
pub use components::ComponentId;
pub use navigation::NavigationState;
pub use screens::Screen;
pub use view_models::{QueueScreenModel, ReaderScreenModel, SettingsScreenModel};

#[derive(Debug, Default)]
pub struct AppRuntime {
    navigation: NavigationState,
}

impl AppRuntime {
    pub const fn new() -> Self {
        Self {
            navigation: NavigationState::new(),
        }
    }

    pub fn tick(&mut self) -> Screen {
        self.navigation.active_screen
    }

    pub fn prepare_queue_view(&self) -> QueueScreenModel {
        QueueScreenModel
    }

    pub fn handle_input_gesture(&mut self, _gesture: InputGesture) {}
}
