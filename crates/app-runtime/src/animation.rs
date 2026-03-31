#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum AnimationDescriptor {
    #[default]
    None,
    Instant,
    Slide,
    Pulse,
}
