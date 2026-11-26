//! 剪贴板历史存储：SQLite（系统自带 libsqlite3，不增加包体）
//! 图片原图与缩略图以 PNG 文件存放在同目录的 images/ 下，库里只存文件名。
//!
//! 数据目录可以放到网盘做多机同步：这时改用 DELETE 日志模式（不产生 -wal/-shm 文件），
//! 并在每次读取前检查数据库文件是否被同步工具整体替换（inode 变化），是则重新打开。

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Image,
    Files,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Image => "image",
            Kind::Files => "files",
        }
    }
    fn parse(s: &str) -> Kind {
        match s {
            "image" => Kind::Image,
            "files" => Kind::Files,
            _ => Kind::Text,
        }
    }
}

/// 列表筛选
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Kind(Kind),
    Favorite,
}

impl Filter {
    pub fn parse(s: &str) -> Filter {
        match s {
            "text" => Filter::Kind(Kind::Text),
            "image" => Filter::Kind(Kind::Image),
            "files" => Filter::Kind(Kind::Files),
            "favorite" => Filter::Favorite,
            _ => Filter::All,
        }
    }
}

/// 一条记录。`content`：文本原文 / 图片文件名（hash）/ 文件路径以换行分隔
#[derive(Debug, Clone)]
pub struct Clip {
    pub id: i64,
    pub kind: Kind,
    pub content: String,
    /// 图片为 "宽×高"，文本为字符数，文件为个数
    pub meta: String,
    pub source_app: Option<String>,
    pub favorite: bool,
    pub tags: Vec<String>,
    pub used_at: i64,
    pub has_ocr: bool,
}

pub struct NewClip<'a> {
    pub kind: Kind,
    pub content: &'a str,
    pub hash: &'a str,
    pub meta: &'a str,
    pub source_app: Option<&'a str>,
    pub source_bundle: Option<&'a str>,
}

const COLUMNS: &str =
    "id, kind, content, meta, source_app, pinned, tags, used_at, ocr IS NOT NULL";

pub struct Store {
    conn: Connection,
    pub dir: PathBuf,
    pub images_dir: PathBuf,
    synced: bool,
    inode: u64,
}

impl Store {
    /// `synced`：数据目录位于网盘等同步目录
    pub fn open(dir: &Path, synced: bool) -> rusqlite::Result<Self> {
        let images_dir = dir.join("images");
        let _ = std::fs::create_dir_all(&images_dir);
        let db = dir.join("clipboard.db");
        let conn = Connection::open(&db)?;
        conn.busy_timeout(std::time::Duration::from_secs(3))?;
        conn.execute_batch(if synced {
            "PRAGMA journal_mode = DELETE;"
        } else {
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;"
        })?;
        migrate(&conn)?;
        Ok(Self {
            inode: std::fs::metadata(&db).map_or(0, |m| m.ino()),
            conn,
            dir: dir.to_path_buf(),
            images_dir,
            synced,
        })
    }

    pub fn db_path(&self) -> PathBuf {
        self.dir.join("clipboard.db")
    }

    /// 同步工具替换了数据库文件时重新打开，读到其他电脑的新记录
    fn ensure_fresh(&mut self) {
        let ino = std::fs::metadata(self.db_path()).map_or(0, |m| m.ino());
        if ino != 0 && ino != self.inode {
            if let Ok(fresh) = Store::open(&self.dir, self.synced) {
                *self = fresh;
            }
        }
    }

    /// 新增；内容相同（hash 相同）则只把它移到最前
    pub fn upsert(&mut self, c: &NewClip) -> rusqlite::Result<i64> {
        self.ensure_fresh();
        let t = now();
        self.conn.query_row(
            "INSERT INTO clips (kind, content, hash, meta, source_app, source_bundle, created_at, used_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
             ON CONFLICT(hash) DO UPDATE SET
               used_at = excluded.used_at,
               source_app = COALESCE(excluded.source_app, source_app),
               source_bundle = COALESCE(excluded.source_bundle, source_bundle)
             RETURNING id",
            params![
                c.kind.as_str(),
                c.content,
                c.hash,
                c.meta,
                c.source_app,
                c.source_bundle,
                t
            ],
            |r| r.get(0),
        )
    }

    /// 查询列表。`query` 匹配文本内容、图片的 OCR 文字、文件路径和来源应用
    pub fn list(&mut self, query: &str, filter: Filter, tag: Option<&str>, limit: u32) -> rusqlite::Result<Vec<Clip>> {
        self.ensure_fresh();
        let q = escape_like(query.trim());
        let (kind, fav) = match filter {
            Filter::All => ("", 0),
            Filter::Kind(k) => (k.as_str(), 0),
            Filter::Favorite => ("", 1),
        };
        let tag = tag.map(str::trim).unwrap_or("");
        let sql = format!(
            "SELECT {COLUMNS} FROM clips
             WHERE (?1 = '' OR (kind != 'image' AND content LIKE '%' || ?1 || '%' ESCAPE '\\')
                    OR ocr_text LIKE '%' || ?1 || '%' ESCAPE '\\'
                    OR source_app LIKE '%' || ?1 || '%' ESCAPE '\\')
               AND (?2 = '' OR kind = ?2)
               AND (?3 = 0 OR pinned = 1)
               AND (?4 = '' OR (',' || tags || ',') LIKE '%,' || ?4 || ',%')
             ORDER BY used_at DESC, id DESC
             LIMIT ?5"
        );
        let mut stmt = self.conn.prepare_cached(&sql)?;
        let rows = stmt.query_map(params![q, kind, fav, tag, limit], row_to_clip)?;
        rows.collect()
    }

    /// 默认排序下的第 n 条（从 1 开始）
    pub fn nth(&mut self, n: u32) -> rusqlite::Result<Option<Clip>> {
        self.ensure_fresh();
        self.conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM clips ORDER BY used_at DESC, id DESC LIMIT 1 OFFSET ?1"),
                [n.saturating_sub(1)],
                row_to_clip,
            )
            .optional()
    }

    pub fn get(&mut self, id: i64) -> rusqlite::Result<Option<Clip>> {
        self.ensure_fresh();
        self.conn
            .query_row(&format!("SELECT {COLUMNS} FROM clips WHERE id = ?1"), [id], row_to_clip)
            .optional()
    }

    /// 收藏中出现过的全部标签
    pub fn tags(&mut self) -> rusqlite::Result<Vec<String>> {
        self.ensure_fresh();
        let mut stmt = self
            .conn
            .prepare("SELECT tags FROM clips WHERE pinned = 1 AND tags != ''")?;
        let mut all: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))?
            .filter_map(Result::ok)
            .flat_map(|t| split_tags(&t))
            .collect();
        all.sort();
        all.dedup();
        Ok(all)
    }

    pub fn touch(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE clips SET used_at = ?1 WHERE id = ?2", params![now(), id])
            .map(|_| ())
    }

    pub fn set_favorite(&self, id: i64, favorite: bool) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE clips SET pinned = ?1 WHERE id = ?2", params![favorite, id])
            .map(|_| ())
    }

    /// 设置标签（会自动收藏，因为标签只用于收藏内容）
    pub fn set_tags(&self, id: i64, tags: &[String]) -> rusqlite::Result<()> {
        let mut clean: Vec<String> = tags
            .iter()
            .map(|t| t.replace(',', " ").trim().to_string())
            .filter(|t| !t.is_empty())
            .collect();
        clean.dedup();
        self.conn
            .execute(
                "UPDATE clips SET tags = ?1, pinned = CASE WHEN ?1 != '' THEN 1 ELSE pinned END WHERE id = ?2",
                params![clean.join(","), id],
            )
            .map(|_| ())
    }

    pub fn ocr(&self, id: i64) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT ocr FROM clips WHERE id = ?1", [id], |r| r.get(0))
            .optional()
            .map(Option::flatten)
    }

    pub fn set_ocr(&self, id: i64, json: &str, text: &str) -> rusqlite::Result<()> {
        self.conn
            .execute(
                "UPDATE clips SET ocr = ?1, ocr_text = ?2 WHERE id = ?3",
                params![json, text, id],
            )
            .map(|_| ())
    }

    pub fn delete(&mut self, id: i64) -> rusqlite::Result<()> {
        if let Some(c) = self.get(id)? {
            self.remove_files(&[c]);
        }
        self.conn.execute("DELETE FROM clips WHERE id = ?1", [id]).map(|_| ())
    }

    /// 清理：超出条数上限、或超过保存天数的非收藏记录
    pub fn prune(&self, max_items: u32, max_days: u32) -> rusqlite::Result<()> {
        let mut old = self.collect(
            &format!(
                "SELECT {COLUMNS} FROM clips WHERE pinned = 0
                 ORDER BY used_at DESC, id DESC LIMIT -1 OFFSET ?1"
            ),
            max_items as i64,
        )?;
        if max_days > 0 {
            let before = now() - max_days as i64 * 86_400;
            old.extend(self.collect(
                &format!("SELECT {COLUMNS} FROM clips WHERE pinned = 0 AND used_at < ?1"),
                before,
            )?);
        }
        self.delete_all(old)
    }

    /// 清空全部非收藏记录
    pub fn clear(&self) -> rusqlite::Result<()> {
        let all = self.collect(
            &format!("SELECT {COLUMNS} FROM clips WHERE pinned = 0 AND ?1 = ?1"),
            0,
        )?;
        self.delete_all(all)
    }

    /// 完整复制到另一个目录（迁移数据目录时使用）
    pub fn copy_to(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir.join("images")).map_err(|e| e.to_string())?;
        let target = dir.join("clipboard.db");
        self.conn
            .execute("VACUUM INTO ?1", [target.to_string_lossy()])
            .map_err(|e| format!("复制数据库失败: {e}"))?;
        if let Ok(rd) = std::fs::read_dir(&self.images_dir) {
            for e in rd.flatten() {
                let _ = std::fs::copy(e.path(), dir.join("images").join(e.file_name()));
            }
        }
        Ok(())
    }

    fn collect(&self, sql: &str, arg: i64) -> rusqlite::Result<Vec<Clip>> {
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map([arg], row_to_clip)?;
        rows.collect()
    }

    fn delete_all(&self, mut clips: Vec<Clip>) -> rusqlite::Result<()> {
        if clips.is_empty() {
            return Ok(());
        }
        clips.sort_by_key(|c| c.id);
        clips.dedup_by_key(|c| c.id);
        let tx = self.conn.unchecked_transaction()?;
        for c in &clips {
            tx.execute("DELETE FROM clips WHERE id = ?1", [c.id])?;
        }
        tx.commit()?;
        self.remove_files(&clips);
        Ok(())
    }

    fn remove_files(&self, clips: &[Clip]) {
        for c in clips.iter().filter(|c| c.kind == Kind::Image) {
            let _ = std::fs::remove_file(self.images_dir.join(format!("{}.png", c.content)));
            let _ = std::fs::remove_file(self.images_dir.join(format!("{}_t.png", c.content)));
        }
    }
}

/// 数据库结构版本迁移（PRAGMA user_version）
fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS clips (
           id            INTEGER PRIMARY KEY,
           kind          TEXT    NOT NULL,
           content       TEXT    NOT NULL,
           hash          TEXT    NOT NULL UNIQUE,
           meta          TEXT    NOT NULL DEFAULT '',
           source_app    TEXT,
           source_bundle TEXT,
           pinned        INTEGER NOT NULL DEFAULT 0,
           created_at    INTEGER NOT NULL,
           used_at       INTEGER NOT NULL
         );",
    )?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version < 1 {
        // v1：收藏标签、OCR 结果缓存（ocr_text 参与搜索）；排序不再置顶收藏
        conn.execute_batch(
            "ALTER TABLE clips ADD COLUMN tags TEXT NOT NULL DEFAULT '';
             ALTER TABLE clips ADD COLUMN ocr TEXT;
             ALTER TABLE clips ADD COLUMN ocr_text TEXT;
             DROP INDEX IF EXISTS idx_clips_order;
             CREATE INDEX IF NOT EXISTS idx_clips_used ON clips (used_at DESC);
             CREATE INDEX IF NOT EXISTS idx_clips_fav ON clips (pinned, used_at DESC);
             PRAGMA user_version = 1;",
        )?;
    }
    Ok(())
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

fn split_tags(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn row_to_clip(r: &rusqlite::Row) -> rusqlite::Result<Clip> {
    Ok(Clip {
        id: r.get(0)?,
        kind: Kind::parse(&r.get::<_, String>(1)?),
        content: r.get(2)?,
        meta: r.get(3)?,
        source_app: r.get(4)?,
        favorite: r.get(5)?,
        tags: split_tags(&r.get::<_, String>(6)?),
        used_at: r.get(7)?,
        has_ocr: r.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("wtools-clip-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Store::open(&dir, false).unwrap()
    }

    fn text<'a>(s: &'a str, hash: &'a str) -> NewClip<'a> {
        NewClip {
            kind: Kind::Text,
            content: s,
            hash,
            meta: "",
            source_app: Some("Test"),
            source_bundle: None,
        }
    }

    #[test]
    fn dedupe_search_favorite_prune() {
        let mut s = tmp_store("a");
        let a = s.upsert(&text("hello world", "h1")).unwrap();
        let b = s.upsert(&text("100% 完成", "h2")).unwrap();
        assert_eq!(s.upsert(&text("hello world", "h1")).unwrap(), a);
        assert_eq!(s.list("", Filter::All, None, 10).unwrap().len(), 2);
        // LIKE 通配符被转义
        assert_eq!(s.list("%", Filter::All, None, 10).unwrap().len(), 1);
        assert_eq!(s.list("完成", Filter::All, None, 10).unwrap()[0].id, b);
        // 收藏 + 标签
        s.set_tags(a, &["工作".into(), "常用".into()]).unwrap();
        // tags() 按 Unicode 排序：工(U+5DE5) < 常(U+5E38)
        assert_eq!(s.tags().unwrap(), vec!["工作".to_string(), "常用".to_string()]);
        assert_eq!(s.list("", Filter::Favorite, None, 10).unwrap().len(), 1);
        assert_eq!(s.list("", Filter::All, Some("工作"), 10).unwrap()[0].id, a);
        assert!(s.list("", Filter::All, Some("工"), 10).unwrap().is_empty());
        // 收藏不会被清理
        s.prune(0, 0).unwrap();
        let left = s.list("", Filter::All, None, 10).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, a);
    }

    #[test]
    fn ocr_text_is_searchable_and_nth() {
        let mut s = tmp_store("b");
        let id = s
            .upsert(&NewClip { kind: Kind::Image, content: "abc", hash: "i1", meta: "1×1", source_app: None, source_bundle: None })
            .unwrap();
        s.upsert(&text("later", "t1")).unwrap();
        s.set_ocr(id, "{}", "发票号码 12345").unwrap();
        assert_eq!(s.list("发票", Filter::Kind(Kind::Image), None, 10).unwrap()[0].id, id);
        assert!(s.get(id).unwrap().unwrap().has_ocr);
        assert_eq!(s.nth(1).unwrap().unwrap().content, "later");
        assert_eq!(s.nth(2).unwrap().unwrap().id, id);
    }

    #[test]
    fn prune_by_days() {
        let mut s = tmp_store("c");
        let id = s.upsert(&text("old", "o1")).unwrap();
        s.conn.execute("UPDATE clips SET used_at = ?1 WHERE id = ?2", params![now() - 10 * 86_400, id]).unwrap();
        s.upsert(&text("new", "n1")).unwrap();
        s.prune(1000, 7).unwrap();
        let left = s.list("", Filter::All, None, 10).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].content, "new");
    }
}

#[cfg(test)]
mod migrate_real {
    /// `WTOOLS_MIGRATE_DIR=<目录> cargo test migrate_real -- --ignored`
    #[test]
    #[ignore]
    fn migrate_real() {
        let dir = std::env::var("WTOOLS_MIGRATE_DIR").unwrap();
        let mut s = super::Store::open(std::path::Path::new(&dir), false).unwrap();
        let n = s.list("", super::Filter::All, None, 100).unwrap().len();
        println!("迁移后读取 {n} 条");
    }
}
