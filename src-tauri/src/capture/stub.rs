//! Everywhere that is not Windows.
//!
//! This exists so the module above is COMPILED AND TESTED on the Linux box the
//! app is developed on. `check.sh` type-checks in a container and never looks
//! inside `#[cfg(windows)]`, so without a stub every line of bucketing, naming,
//! compositing and sealing would reach CI unexamined. The data is invented; the
//! shape is exactly what Windows returns.

use std::path::Path;

use super::{Probe, Shot, Source};

pub struct Screens;

impl Screens {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }
}

impl Source for Screens {
    fn probe(&self) -> Result<Probe, String> {
        Ok(Probe {
            idle_secs: 0,
            locked: false,
            process: "stub".into(),
            title: "not Windows".into(),
        })
    }

    fn shots(&self) -> Result<Vec<Shot>, String> {
        Ok(vec![Shot {
            x: 0,
            y: 0,
            width: 4,
            height: 2,
            rgba: vec![0; 4 * 2 * 4],
        }])
    }

    fn free_bytes(&self, _path: &Path) -> u64 {
        u64::MAX
    }
}
