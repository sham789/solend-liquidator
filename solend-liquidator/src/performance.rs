use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub enum Setting {
    Ms,
    Micros,
}

impl Setting {
    fn format(&self) -> String {
        match self {
            Self::Ms => "ms".to_string(),
            Self::Micros => "micros".to_string(),
        }
    }
}

impl Default for Setting {
    fn default() -> Self {
        Self::Ms
    }
}

#[derive(Default, Clone, Debug)]
pub struct PerformanceMeter<'a> {
    points_table: HashMap<&'a str, u128>,
    points_order: Vec<&'a str>,
    setting: Setting,
}

pub fn current_timestamp_ms() -> u128 {
    let point_a = SystemTime::now();
    let point_a = point_a.duration_since(UNIX_EPOCH).unwrap();
    point_a.as_millis()
}

pub fn current_timestamp_micros() -> u128 {
    let point_a = SystemTime::now();
    let point_a = point_a.duration_since(UNIX_EPOCH).unwrap();
    point_a.as_micros()
}

unsafe impl Send for PerformanceMeter<'_> {}
unsafe impl Sync for PerformanceMeter<'_> {}

impl<'a> PerformanceMeter<'a> {
    pub fn new(setting: Setting) -> Self {
        Self {
            setting,
            ..Self::default()
        }
    }

    pub fn timestamp(&self) -> u128 {
        match self.setting {
            Setting::Ms => current_timestamp_ms(),
            Setting::Micros => current_timestamp_micros(),
        }
    }

    pub fn add_point(&mut self, tag: &'a str) {
        self.points_table.insert(tag, self.timestamp());
        self.points_order.push(tag);
    }

    pub fn clear(&mut self) {
        self.points_table = HashMap::new();
        self.points_order = vec![];
    }

    pub fn measure(&self) {
        let n = self.points_order.len();

        for i in 0..n - 1 {
            let (a, b) = (self.points_order[i], self.points_order[i + 1]);
            let current = self.points_table.get(a).unwrap();
            let next = self.points_table.get(b).unwrap();

            println!(
                " 🕒 point {:} to {:} took {:} {:}",
                a,
                b,
                next - current,
                self.setting.format()
            );
        }

        // println!("🔺 🕒 longest point: {:} to {:} (took {:} ms)", a, b, next - current);
    }
}
