//! 应用图标：自定义协议 `wicon://localhost/<路径>`，按需生成 64×64 PNG 并缓存到磁盘
//!
//! 只有真正显示在结果列表里的图标才会生成，避免启动时为几百个应用全部渲染。

use std::{
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::OnceLock,
    time::UNIX_EPOCH,
};
use tauri::{
    http::{header, Request, Response, StatusCode},
    UriSchemeContext, UriSchemeResponder, Wry,
};

pub const SCHEME: &str = "wicon";
const SIZE: isize = 64; // 32pt @2x

static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn init(cache_dir: PathBuf) {
    let dir = cache_dir.join("icons");
    let _ = std::fs::create_dir_all(&dir);
    let _ = CACHE_DIR.set(dir);
}

pub fn url_for(path: &Path) -> String {
    format!("{SCHEME}://localhost/{}", percent_encode(&path.to_string_lossy()))
}

/// 协议处理：在后台线程生成，不阻塞主线程
pub fn handle(_ctx: UriSchemeContext<'_, Wry>, req: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let raw = req.uri().path().trim_start_matches('/').to_string();
    std::thread::spawn(move || {
        let path = PathBuf::from(format!("/{}", percent_decode(&raw)));
        let res = match load(&path) {
            Some(png) => Response::builder()
                .header(header::CONTENT_TYPE, "image/png")
                .header(header::CACHE_CONTROL, "max-age=86400")
                .body(png),
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Vec::new()),
        };
        responder.respond(res.unwrap_or_else(|_| Response::new(Vec::new())));
    });
}

fn load(path: &Path) -> Option<Vec<u8>> {
    if path.extension().is_none_or(|x| x != "app") {
        return None;
    }
    let cache = cache_file(path);
    if let Some(c) = &cache {
        if let Ok(bytes) = std::fs::read(c) {
            return Some(bytes);
        }
    }
    let png = render(path)?;
    if let Some(c) = cache {
        let _ = std::fs::write(c, &png);
    }
    Some(png)
}

/// 缓存键包含修改时间：应用更新换图标后自动失效
fn cache_file(path: &Path) -> Option<PathBuf> {
    let dir = CACHE_DIR.get()?;
    let mtime = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    mtime.hash(&mut h);
    Some(dir.join(format!("{:016x}.png", h.finish())))
}

#[cfg(target_os = "macos")]
fn render(path: &Path) -> Option<Vec<u8>> {
    use objc2::{rc::autoreleasepool, AnyThread};
    use objc2_app_kit::{
        NSBitmapImageFileType, NSBitmapImageRep, NSCompositingOperation, NSDeviceRGBColorSpace,
        NSGraphicsContext, NSWorkspace,
    };
    use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString};

    autoreleasepool(|_| unsafe {
        let icon = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path.to_str()?));
        let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            SIZE,
            SIZE,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            0,
            0,
        )?;
        let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&ctx));
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(SIZE as f64, SIZE as f64));
        icon.drawInRect_fromRect_operation_fraction(
            rect,
            NSRect::ZERO,
            NSCompositingOperation::Copy,
            1.0,
        );
        ctx.flushGraphics();
        NSGraphicsContext::restoreGraphicsState_class();
        let data = rep.representationUsingType_properties(
            NSBitmapImageFileType::PNG,
            &NSDictionary::new(),
        )?;
        Some(data.to_vec())
    })
}

#[cfg(not(target_os = "macos"))]
fn render(_path: &Path) -> Option<Vec<u8>> {
    None
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.trim_start_matches('/').bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(v) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
            {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_roundtrip() {
        let p = "/Applications/Visual Studio Code.app";
        let url = url_for(Path::new(p));
        assert_eq!(url, "wicon://localhost/Applications/Visual%20Studio%20Code.app");
        let raw = url.trim_start_matches("wicon://localhost/");
        assert_eq!(format!("/{}", percent_decode(raw)), p);
        assert_eq!(percent_decode(&percent_encode("/应用/微信.app")), "应用/微信.app");
    }
}

#[cfg(test)]
mod render_tests {
    /// `cargo test render_real_icon -- --ignored`，输出到 $WTOOLS_ICON_OUT
    #[test]
    #[ignore]
    fn render_real_icon() {
        let t = std::time::Instant::now();
        let png = super::render(std::path::Path::new("/System/Applications/System Settings.app"))
            .expect("render failed");
        println!("渲染耗时 {:?}, {} 字节", t.elapsed(), png.len());
        if let Ok(out) = std::env::var("WTOOLS_ICON_OUT") {
            std::fs::write(out, png).unwrap();
        }
    }
}
