//! Fallback probe for non-Windows platforms: reports monitoring as
//! unavailable rather than silently failing (spec §41). Demo Mode still
//! exercises the full pipeline on any OS.

use super::{ActivityProbe, ProbeReading};

pub struct NoopProbe;

impl ActivityProbe for NoopProbe {
    fn read(&mut self) -> ProbeReading {
        ProbeReading::Unavailable(
            "Desktop monitoring is implemented for Windows in this build. Enable Demo Mode to simulate activity.".into(),
        )
    }
}
