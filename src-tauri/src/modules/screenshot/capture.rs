//! 屏幕截取（CoreGraphics）：鼠标所在显示器整屏截图、窗口位置列表、屏幕录制权限

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::AnyThread;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString};
use serde::Serialize;
use std::ffi::c_void;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

type CGImageRef = *mut c_void;

#[link(name = "CoreGraphics", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    fn CGEventGetLocation(event: *const c_void) -> CGPoint;
    fn CGGetDisplaysWithPoint(point: CGPoint, max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayCreateImage(display: u32) -> CGImageRef;
    fn CGImageCreateWithImageInRect(image: CGImageRef, rect: CGRect) -> CGImageRef;
    fn CGImageGetWidth(image: CGImageRef) -> usize;
    fn CGImageGetHeight(image: CGImageRef) -> usize;
    fn CGImageGetBytesPerRow(image: CGImageRef) -> usize;
    fn CGImageGetBitsPerPixel(image: CGImageRef) -> usize;
    fn CGImageGetBitmapInfo(image: CGImageRef) -> u32;
    fn CGImageGetDataProvider(image: CGImageRef) -> *mut c_void;
    fn CGDataProviderCopyData(provider: *mut c_void) -> *mut c_void;
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFDataGetLength(data: *const c_void) -> isize;
    fn CFRelease(obj: *const c_void);
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *mut c_void;
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
}

/// 是否已获得「屏幕录制」权限。没有权限时截图里只有桌面和菜单栏，看不到其他应用的窗口。
pub fn has_permission() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// 请求权限：首次调用会弹出系统提示，并把 wtools 加入系统设置的列表
pub fn request_permission() {
    unsafe {
        CGRequestScreenCaptureAccess();
    }
}

/// 一个显示器：ID 与位置（全局坐标，单位为点，原点在主屏左上角）
#[derive(Debug, Clone, Copy)]
pub struct Display {
    pub id: u32,
    pub bounds: CGRect,
}

/// 鼠标所在的显示器
pub fn display_under_cursor() -> Display {
    unsafe {
        let ev = CGEventCreate(std::ptr::null());
        let p = if ev.is_null() { CGPoint::default() } else { CGEventGetLocation(ev) };
        if !ev.is_null() {
            CFRelease(ev);
        }
        let mut id = 0u32;
        let mut count = 0u32;
        if CGGetDisplaysWithPoint(p, 1, &mut id, &mut count) != 0 || count == 0 {
            id = CGMainDisplayID();
        }
        Display { id, bounds: CGDisplayBounds(id) }
    }
}

/// 主屏高度（点），用于把左上角原点的坐标换算成 AppKit 的左下角原点坐标
pub fn main_display_height() -> f64 {
    unsafe { CGDisplayBounds(CGMainDisplayID()).size.height }
}

/// 一次截图：保留原始 CGImage（用于裁剪识别文字），并转换出 RGBA 像素（直接交给前端画布）
pub struct Capture {
    image: CGImageRef,
    pub width: usize,
    pub height: usize,
}

// CGImage 是不可变对象，可以跨线程使用
unsafe impl Send for Capture {}
unsafe impl Sync for Capture {}

impl Drop for Capture {
    fn drop(&mut self) {
        unsafe { CFRelease(self.image) }
    }
}

impl Capture {
    pub fn display(d: &Display) -> Result<Self, String> {
        let image = unsafe { CGDisplayCreateImage(d.id) };
        if image.is_null() {
            return Err("截取屏幕失败".into());
        }
        let (width, height) = unsafe { (CGImageGetWidth(image), CGImageGetHeight(image)) };
        Ok(Self { image, width, height })
    }

    /// 转成 RGBA（前端 ImageData 的格式）。系统截图通常是 BGRA 小端序，其他格式按位信息换算。
    pub fn rgba(&self) -> Result<Vec<u8>, String> {
        unsafe {
            if CGImageGetBitsPerPixel(self.image) != 32 {
                return Err("不支持的屏幕像素格式".into());
            }
            let info = CGImageGetBitmapInfo(self.image);
            let bpr = CGImageGetBytesPerRow(self.image);
            let data = CGDataProviderCopyData(CGImageGetDataProvider(self.image));
            if data.is_null() {
                return Err("读取截图像素失败".into());
            }
            let src = std::slice::from_raw_parts(CFDataGetBytePtr(data), CFDataGetLength(data) as usize);
            const BYTE_ORDER_MASK: u32 = 0x7000;
            const LITTLE_32: u32 = 0x2000;
            let alpha = info & 0x1f;
            // alpha 在前：PremultipliedFirst(2) / First(4) / NoneSkipFirst(6)
            let alpha_first = matches!(alpha, 2 | 4 | 6);
            let little = info & BYTE_ORDER_MASK == LITTLE_32;
            // 每种布局下 R、G、B 在 4 字节里的位置
            let (r, g, b) = match (little, alpha_first) {
                (true, true) => (2, 1, 0),   // BGRA
                (true, false) => (3, 2, 1),  // ABGR
                (false, true) => (1, 2, 3),  // ARGB
                (false, false) => (0, 1, 2), // RGBA
            };
            let (w, h) = (self.width, self.height);
            let mut out = vec![0u8; w * h * 4];
            for y in 0..h {
                let row = &src[y * bpr..y * bpr + w * 4];
                let dst = &mut out[y * w * 4..(y + 1) * w * 4];
                for (s, d) in row.chunks_exact(4).zip(dst.chunks_exact_mut(4)) {
                    d[0] = s[r];
                    d[1] = s[g];
                    d[2] = s[b];
                    d[3] = 255;
                }
            }
            CFRelease(data);
            Ok(out)
        }
    }

    /// 裁剪一块区域（像素坐标）并编码为 PNG
    pub fn crop_png(&self, x: f64, y: f64, w: f64, h: f64) -> Result<Vec<u8>, String> {
        autoreleasepool(|_| unsafe {
            let rect = CGRect { origin: CGPoint { x, y }, size: CGSize { width: w, height: h } };
            let part = CGImageCreateWithImageInRect(self.image, rect);
            if part.is_null() {
                return Err("裁剪区域无效".into());
            }
            let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &*(part as *const _));
            CFRelease(part);
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
                .map(|d| d.to_vec())
                .ok_or_else(|| "编码图片失败".into())
        })
    }
}

/// 屏幕上的一个窗口（相对显示器左上角，单位为点）
#[derive(Debug, Clone, Serialize)]
pub struct WinRect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// 显示器上可见的窗口，从前到后排列。用于鼠标悬停时自动选中窗口。
/// 只读取位置和层级，不需要额外权限。
pub fn windows_on(d: &Display) -> Vec<WinRect> {
    const ON_SCREEN_ONLY: u32 = 1 << 0;
    const EXCLUDE_DESKTOP: u32 = 1 << 4;
    let own_pid = std::process::id() as i64;
    let b = d.bounds;
    autoreleasepool(|_| {
        let raw = unsafe { CGWindowListCopyWindowInfo(ON_SCREEN_ONLY | EXCLUDE_DESKTOP, 0) };
        if raw.is_null() {
            return Vec::new();
        }
        // CFArray 与 NSArray 可以直接互换；Copy 函数返回 +1 引用，交给 Retained 释放
        let Some(list) = (unsafe { Retained::from_raw(raw as *mut NSArray<AnyObject>) }) else {
            return Vec::new();
        };
        let num = |dict: &NSDictionary<AnyObject, AnyObject>, key: &str| -> Option<f64> {
            let k = NSString::from_str(key);
            let v = dict.objectForKey(k.as_ref())?;
            v.downcast_ref::<NSNumber>().map(|n| n.doubleValue())
        };
        let mut out = Vec::new();
        for item in list.iter() {
            let Some(dict) = item.downcast_ref::<NSDictionary>() else { continue };
            let layer = num(dict, "kCGWindowLayer").unwrap_or(0.0) as i64;
            // 普通窗口、浮动面板、Dock、菜单栏（0..=25）；更高层级是系统覆盖层
            if !(0..=25).contains(&layer) {
                continue;
            }
            if num(dict, "kCGWindowOwnerPID").unwrap_or(0.0) as i64 == own_pid {
                continue;
            }
            if num(dict, "kCGWindowAlpha").unwrap_or(1.0) <= 0.01 {
                continue;
            }
            let k = NSString::from_str("kCGWindowBounds");
            let Some(bounds) = dict.objectForKey(k.as_ref()) else { continue };
            let Some(bounds) = bounds.downcast_ref::<NSDictionary>() else { continue };
            let (Some(x), Some(y), Some(w), Some(h)) = (
                num(bounds, "X"),
                num(bounds, "Y"),
                num(bounds, "Width"),
                num(bounds, "Height"),
            ) else {
                continue;
            };
            // 裁剪到当前显示器，换算成相对坐标
            let x0 = x.max(b.origin.x);
            let y0 = y.max(b.origin.y);
            let x1 = (x + w).min(b.origin.x + b.size.width);
            let y1 = (y + h).min(b.origin.y + b.size.height);
            if x1 - x0 < 8.0 || y1 - y0 < 8.0 {
                continue;
            }
            out.push(WinRect { x: x0 - b.origin.x, y: y0 - b.origin.y, w: x1 - x0, h: y1 - y0 });
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 本机实测：`cargo test real_capture -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_capture() {
        let d = display_under_cursor();
        println!("显示器 {:?} 权限 {}", d.bounds, has_permission());
        let t = std::time::Instant::now();
        let c = Capture::display(&d).unwrap();
        let t1 = t.elapsed();
        let px = c.rgba().unwrap();
        println!("{}x{} 截图 {t1:?}，转 RGBA 共 {:?}，{} MB", c.width, c.height, t.elapsed(), px.len() / 1_000_000);
        let wins = windows_on(&d);
        println!("窗口 {} 个，前 5 个：{:?}", wins.len(), &wins[..wins.len().min(5)]);
        let png = c.crop_png(0.0, 0.0, 200.0, 100.0).unwrap();
        assert!(png.starts_with(b"\x89PNG"));
    }
}
