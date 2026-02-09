pub mod book_font;
pub mod rsvp;

use ls027b7dh01::FrameBuffer;
use readily_core::render::Screen;

pub trait FrameRenderer {
    fn render(&mut self, screen: Screen<'_>, frame: &mut FrameBuffer);
}
