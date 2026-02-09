use core::convert::Infallible;

use embedded_graphics_core::{
    Pixel,
    draw_target::DrawTarget,
    geometry::{OriginDimensions, Size},
    pixelcolor::BinaryColor,
};

use crate::{FrameBuffer, protocol};

impl DrawTarget for FrameBuffer {
    type Color = BinaryColor;
    type Error = Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x < 0 || point.y < 0 {
                continue;
            }

            let x = point.x as usize;
            let y = point.y as usize;
            let _ = self.set_pixel(x, y, color.is_on());
        }

        Ok(())
    }
}

impl OriginDimensions for FrameBuffer {
    fn size(&self) -> Size {
        Size::new(protocol::WIDTH as u32, protocol::HEIGHT as u32)
    }
}
