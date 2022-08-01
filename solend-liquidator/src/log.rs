use log::{info, trace, warn};
use log::{Level, Metadata, Record};

#[derive(Debug, Clone, Copy, Default)]
pub struct Logger {
    enabled: bool,
}

impl log::Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        // metadata.level() <= Level::Info
        true
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            println!("{} - {}", record.level(), record.args());
        }
    }

    fn flush(&self) {}
}

impl Logger {
    pub fn new() -> Self {
        Self { enabled: true }
    }

    // pub fn from(enabled: bool) -> Self {
    //     Self { enabled }
    // }

    // pub fn disable(&mut self) {
    //     self.enabled = false;
    // }

    // pub fn enable(&mut self) {
    //     self.enabled = true;
    // }

    // pub fn trace<T: std::fmt::Display>(&self, msg: T) {
    //     trace!("{:}", msg);
    //     if self.enabled {
    //     }
    // }

    // pub fn info<T: std::fmt::Display>(&self, msg: T) {
    //     info!("{:}", msg);
    //     if self.enabled {
    //     }
    // }

    // pub fn warn<T: std::fmt::Display>(&self, msg: T) {
    //     warn!("{:}", msg);
    //     if self.enabled {
    //     }
    // }
}

struct PerformanceLogger {
    inner: Logger,
}
