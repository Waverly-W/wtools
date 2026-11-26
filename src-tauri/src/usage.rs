//! 使用频率：记录每个结果被执行的次数与最近时间，用于搜索排序加权

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct Record {
    count: u32,
    last: u64,
}

pub struct Usage {
    path: PathBuf,
    records: Mutex<HashMap<String, Record>>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Usage {
    pub fn load(path: PathBuf) -> Self {
        let records = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        Self {
            path,
            records: Mutex::new(records),
        }
    }

    pub fn record(&self, id: &str) {
        let snapshot = {
            let mut r = self.records.lock().unwrap();
            let e = r.entry(id.to_string()).or_default();
            e.count = e.count.saturating_add(1);
            e.last = now();
            serde_json::to_string(&*r).ok()
        };
        // 落盘放到后台，不拖慢执行
        if let Some(data) = snapshot {
            let path = self.path.clone();
            std::thread::spawn(move || {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let tmp = path.with_extension("json.tmp");
                if std::fs::write(&tmp, data).is_ok() {
                    let _ = std::fs::rename(&tmp, &path);
                }
            });
        }
    }

    /// 排序加分：次数取对数（避免常用项永远霸榜），最近 3 天用过的再额外加分
    pub fn boost(&self, id: &str) -> u32 {
        let r = self.records.lock().unwrap();
        let Some(rec) = r.get(id) else { return 0 };
        let freq = ((rec.count as f64).ln_1p() * 60.0) as u32;
        let recent = match now().saturating_sub(rec.last) {
            s if s < 3600 => 80,
            s if s < 86400 => 50,
            s if s < 3 * 86400 => 20,
            _ => 0,
        };
        freq + recent
    }
}
