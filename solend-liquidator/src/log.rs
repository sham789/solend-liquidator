use log::{info, trace, warn};

#[derive(Debug, Clone, Copy, Default)]
pub struct Logger {
    enabled: bool,
}

impl Logger {
    pub fn new() -> Self {
        Self { enabled: true }
    }

    pub fn from(enabled: bool) -> Self {
        Self { enabled }
    }

    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn enable(&mut self) {
        self.enabled = true;
    }

    pub fn trace<T: std::fmt::Display>(&self, msg: T) {
        if self.enabled {
            trace!("{:}", msg);
        }
    }

    pub fn info<T: std::fmt::Display>(&self, msg: T) {
        if self.enabled {
            info!("{:}", msg);
        }
    }

    pub fn warn<T: std::fmt::Display>(&self, msg: T) {
        if self.enabled {
            warn!("{:}", msg);
        }
    }
}
