//! Studio chain: combines all phone microphones into one loud, clear voice.
//! Runs on the PC, so the phone only captures and sends raw channels.

pub mod align;
pub mod filters;
pub mod studio;

pub use studio::Studio;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NoiseLevel {
    Off,
    #[default]
    Medium,
    Strong,
}

impl NoiseLevel {
    pub fn from_index(i: u8) -> Self {
        match i {
            0 => NoiseLevel::Off,
            2 => NoiseLevel::Strong,
            _ => NoiseLevel::Medium,
        }
    }
}
