//! 系统监控（替代 Stats）：菜单栏显示 CPU / 内存 / 磁盘占用
//!
//! 后台线程按间隔采样；渲染和更新菜单栏在主线程，显示内容不变时跳过。
//! 菜单栏图像为模板图（只用透明度），浅色 / 深色菜单栏由系统自动着色。
//! 图像直接生成 NSImage 设到状态栏按钮上：不经 PNG 编解码，且固定使用 sRGB 色彩空间，
//! 避免系统为每张新图各建一份颜色转换缓存（每份约 350KB）。

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{
    image::Image,
    menu::{MenuBuilder, MenuItem, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Wry,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StatsConfig {
    pub enabled: bool,
    pub cpu: bool,
    pub memory: bool,
    pub disk: bool,
    /// 刷新间隔（秒）
    pub interval: u32,
}

impl Default for StatsConfig {
    fn default() -> Self {
        Self { enabled: false, cpu: true, memory: true, disk: true, interval: 2 }
    }
}

const TRAY_ID: &str = "stats";

/// 每次 apply 递增；采样线程发现代数变化即退出
static GENERATION: AtomicU64 = AtomicU64::new(0);

struct MenuItems {
    cpu: MenuItem<Wry>,
    memory: MenuItem<Wry>,
    disk: MenuItem<Wry>,
    /// 上次显示的内容：菜单栏数值、三行菜单文字
    last_image: String,
    last_text: [String; 3],
}

static MENU: Mutex<Option<MenuItems>> = Mutex::new(None);

pub fn apply(app: &AppHandle, cfg: &StatsConfig) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let _ = app.remove_tray_by_id(TRAY_ID);
    *MENU.lock().unwrap() = None;
    if !cfg.enabled || !(cfg.cpu || cfg.memory || cfg.disk) {
        return;
    }
    if let Err(e) = create_tray(app) {
        eprintln!("[stats] 创建菜单栏图标失败: {e}");
        return;
    }
    let app = app.clone();
    let cfg = cfg.clone();
    std::thread::Builder::new()
        .name("wtools-stats".into())
        .spawn(move || run(app, cfg, generation))
        .ok();
}

fn create_tray(app: &AppHandle) -> tauri::Result<()> {
    let cpu = MenuItemBuilder::with_id("stats.cpu", "CPU").enabled(false).build(app)?;
    let memory = MenuItemBuilder::with_id("stats.memory", "内存").enabled(false).build(app)?;
    let disk = MenuItemBuilder::with_id("stats.disk", "磁盘").enabled(false).build(app)?;
    let monitor = MenuItemBuilder::with_id("stats.monitor", "打开活动监视器").build(app)?;
    let settings = MenuItemBuilder::with_id("stats.settings", "系统增强设置…").build(app)?;
    let menu = MenuBuilder::new(app)
        .items(&[&cpu, &memory, &disk])
        .separator()
        .items(&[&monitor, &settings])
        .build()?;
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::new_owned(vec![0; 4], 1, 1))
        .icon_as_template(true)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "stats.monitor" => {
                let _ = std::process::Command::new("open").args(["-b", "com.apple.ActivityMonitor"]).spawn();
            }
            "stats.settings" => {
                let _ = crate::window::open_settings(app);
            }
            _ => {}
        })
        .build(app)?;
    *MENU.lock().unwrap() = Some(MenuItems { cpu, memory, disk, last_image: String::new(), last_text: Default::default() });
    Ok(())
}

#[derive(Debug, Clone, Copy, Default)]
struct Sample {
    cpu: Option<CpuUsage>,
    memory: Option<(u64, u64)>,
    disk: Option<(u64, u64)>,
}

#[derive(Debug, Clone, Copy, Default)]
struct CpuUsage {
    user: f64,
    system: f64,
}

impl CpuUsage {
    fn total(&self) -> f64 {
        self.user + self.system
    }
}

fn run(app: AppHandle, cfg: StatsConfig, generation: u64) {
    let interval = Duration::from_secs(cfg.interval.clamp(1, 60) as u64);
    let mut prev_ticks = if cfg.cpu { sys::cpu_ticks() } else { None };
    let mut disk = None;
    let mut tick: u64 = 0;
    // 先显示一次（CPU 需要两次采样才有差值，首帧短暂等待）
    std::thread::sleep(Duration::from_millis(400));
    loop {
        if GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        let mut s = Sample::default();
        if cfg.cpu {
            let now = sys::cpu_ticks();
            if let (Some(a), Some(b)) = (prev_ticks, now) {
                s.cpu = Some(sys::cpu_usage(a, b));
            }
            prev_ticks = now;
        }
        if cfg.memory {
            s.memory = sys::memory();
        }
        // 磁盘占用变化很慢，每分钟读一次
        if cfg.disk {
            if tick % (60 / interval.as_secs()).max(1) == 0 {
                disk = sys::disk();
            }
            s.disk = disk;
        }
        tick += 1;
        let handle = app.clone();
        let c = cfg.clone();
        let _ = app.run_on_main_thread(move || update(&handle, &c, &s, generation));
        std::thread::sleep(interval);
    }
}

fn pct(used: u64, total: u64) -> f64 {
    if total == 0 { 0.0 } else { used as f64 * 100.0 / total as f64 }
}

fn gb(bytes: u64) -> String {
    let g = bytes as f64 / 1_073_741_824.0;
    if g >= 100.0 { format!("{g:.0} GB") } else { format!("{g:.1} GB") }
}

/// 主线程：重绘菜单栏图像、更新菜单里的详细信息（只更新变化的部分）
fn update(app: &AppHandle, cfg: &StatsConfig, s: &Sample, generation: u64) {
    if GENERATION.load(Ordering::SeqCst) != generation {
        return;
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let mut cols: Vec<(&'static str, String)> = Vec::new();
    if cfg.cpu {
        cols.push(("CPU", s.cpu.map(|c| format!("{:.0}%", c.total())).unwrap_or_else(|| "–".into())));
    }
    if cfg.memory {
        cols.push(("RAM", s.memory.map(|(u, t)| format!("{:.0}%", pct(u, t))).unwrap_or_else(|| "–".into())));
    }
    if cfg.disk {
        cols.push(("SSD", s.disk.map(|(u, t)| format!("{:.0}%", pct(u, t))).unwrap_or_else(|| "–".into())));
    }
    let texts = [
        match s.cpu {
            Some(c) => format!("CPU：{:.0}%（用户 {:.0}% · 系统 {:.0}%）", c.total(), c.user, c.system),
            None => "CPU：–".into(),
        },
        match s.memory {
            Some((u, t)) => format!("内存：{} / {}（{:.0}%）", gb(u), gb(t), pct(u, t)),
            None => "内存：–".into(),
        },
        match s.disk {
            Some((u, t)) => format!("磁盘：已用 {} / {}，可用 {}", gb(u), gb(t), gb(t.saturating_sub(u))),
            None => "磁盘：–".into(),
        },
    ];
    let mut guard = MENU.lock().unwrap();
    let Some(m) = guard.as_mut() else { return };
    let key = cols.iter().map(|c| c.1.as_str()).collect::<Vec<_>>().join("|");
    if m.last_image != key {
        m.last_image = key;
        let _ = tray.with_inner_tray_icon(move |t| {
            let (Some(item), Some(mtm)) = (t.ns_status_item(), objc2::MainThreadMarker::new()) else { return };
            if let (Some(button), Some(img)) = (item.button(mtm), render::mini_widgets(&cols)) {
                button.setImage(Some(&img));
            }
        });
    }
    let items = [&m.cpu, &m.memory, &m.disk];
    for (i, text) in texts.into_iter().enumerate() {
        if m.last_text[i] != text {
            let _ = items[i].set_text(&text);
            m.last_text[i] = text;
        }
    }
}

/// 系统指标采样（mach / sysctl / Foundation）
mod sys {
    use super::CpuUsage;
    use objc2_foundation::{NSArray, NSNumber, NSString, NSURL};

    #[allow(non_camel_case_types)]
    type mach_port_t = u32;

    const HOST_CPU_LOAD_INFO: i32 = 3;
    const HOST_VM_INFO64: i32 = 4;

    #[repr(C)]
    #[derive(Default)]
    struct VmStatistics64 {
        free_count: u32,
        active_count: u32,
        inactive_count: u32,
        wire_count: u32,
        zero_fill_count: u64,
        reactivations: u64,
        pageins: u64,
        pageouts: u64,
        faults: u64,
        cow_faults: u64,
        lookups: u64,
        hits: u64,
        purges: u64,
        purgeable_count: u32,
        speculative_count: u32,
        decompressions: u64,
        compressions: u64,
        swapins: u64,
        swapouts: u64,
        compressor_page_count: u32,
        throttled_count: u32,
        external_page_count: u32,
        internal_page_count: u32,
        total_uncompressed_pages_in_compressor: u64,
    }

    unsafe extern "C" {
        fn mach_host_self() -> mach_port_t;
        fn mach_port_deallocate(task: mach_port_t, name: mach_port_t) -> i32;
        static mach_task_self_: mach_port_t;
        fn host_statistics(host: mach_port_t, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
        fn host_statistics64(host: mach_port_t, flavor: i32, info: *mut i32, count: *mut u32) -> i32;
    }

    fn with_host<T>(f: impl FnOnce(mach_port_t) -> T) -> T {
        unsafe {
            let host = mach_host_self();
            let r = f(host);
            mach_port_deallocate(mach_task_self_, host);
            r
        }
    }

    /// 累计 CPU 时钟数：用户、系统、空闲、nice
    pub fn cpu_ticks() -> Option<[u32; 4]> {
        with_host(|host| {
            let mut info = [0u32; 4];
            let mut count = 4u32;
            let r = unsafe { host_statistics(host, HOST_CPU_LOAD_INFO, info.as_mut_ptr() as *mut i32, &mut count) };
            (r == 0).then_some(info)
        })
    }

    pub fn cpu_usage(a: [u32; 4], b: [u32; 4]) -> CpuUsage {
        let d = |i: usize| b[i].wrapping_sub(a[i]) as f64;
        let (user, system, idle, nice) = (d(0), d(1), d(2), d(3));
        let total = user + system + idle + nice;
        if total <= 0.0 {
            return CpuUsage::default();
        }
        CpuUsage { user: (user + nice) * 100.0 / total, system: system * 100.0 / total }
    }

    fn total_memory() -> u64 {
        let mut v: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        unsafe {
            libc::sysctlbyname(c"hw.memsize".as_ptr(), &mut v as *mut u64 as *mut _, &mut len, std::ptr::null_mut(), 0);
        }
        v
    }

    /// （已用, 总量）字节。「已用」与 Stats / 活动监视器口径一致：活跃 + 非活跃 + 推测 + 联动 + 压缩 − 可清除 − 文件缓存
    pub fn memory() -> Option<(u64, u64)> {
        let total = total_memory();
        let vm = with_host(|host| {
            let mut vm = VmStatistics64::default();
            let mut count = (std::mem::size_of::<VmStatistics64>() / 4) as u32;
            let r = unsafe { host_statistics64(host, HOST_VM_INFO64, &mut vm as *mut _ as *mut i32, &mut count) };
            (r == 0).then_some(vm)
        })?;
        let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
        let pages = vm.active_count as i64 + vm.inactive_count as i64 + vm.speculative_count as i64
            + vm.wire_count as i64
            + vm.compressor_page_count as i64
            - vm.purgeable_count as i64
            - vm.external_page_count as i64;
        Some(((pages.max(0) as u64 * page).min(total), total))
    }

    /// 启动卷（已用, 总量）字节；「可用」含可清除空间，与访达一致
    pub fn disk() -> Option<(u64, u64)> {
        objc2::rc::autoreleasepool(|_| unsafe {
            let url = NSURL::fileURLWithPath(&NSString::from_str("/"));
            let keys = NSArray::from_slice(&[
                objc2_foundation::NSURLVolumeTotalCapacityKey,
                objc2_foundation::NSURLVolumeAvailableCapacityForImportantUsageKey,
            ]);
            let values = url.resourceValuesForKeys_error(&keys).ok()?;
            let num = |k: &NSString| {
                values.objectForKey(k).and_then(|o| o.downcast::<NSNumber>().ok()).map(|n| n.longLongValue().max(0) as u64)
            };
            let total = num(objc2_foundation::NSURLVolumeTotalCapacityKey)?;
            let avail = num(objc2_foundation::NSURLVolumeAvailableCapacityForImportantUsageKey)?;
            Some((total.saturating_sub(avail), total))
        })
    }
}

/// 菜单栏图像：每项上方小字标签、下方数值（与 Stats 的 mini 样式一致）
mod render {
    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::AnyThread;
    use objc2_app_kit::{
        NSColor, NSFont, NSFontAttributeName, NSFontWeightRegular, NSFontWeightSemibold,
        NSForegroundColorAttributeName, NSGraphicsContext, NSImage, NSStringDrawing,
    };
    use objc2_foundation::{NSDictionary, NSPoint, NSSize, NSString};
    use std::ffi::c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorSpaceCreateWithName(name: *const c_void) -> *mut c_void;
        fn CGColorSpaceRelease(cs: *mut c_void);
        static kCGColorSpaceSRGB: *const c_void;
        fn CGBitmapContextCreateImage(ctx: *mut c_void) -> *mut c_void;
        fn CGImageRelease(img: *mut c_void);
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits_per_component: usize,
            bytes_per_row: usize,
            space: *mut c_void,
            bitmap_info: u32,
        ) -> *mut c_void;
        fn CGContextRelease(ctx: *mut c_void);
        fn CGContextScaleCTM(ctx: *mut c_void, sx: f64, sy: f64);
    }

    /// 菜单栏图像高度（点），与 Tauri 托盘图标的显示高度一致
    const HEIGHT: f64 = 18.0;
    const SCALE: f64 = 2.0;
    const GAP: f64 = 6.0;

    fn attrs(font: &NSFont) -> Retained<NSDictionary<NSString, AnyObject>> {
        let color = NSColor::blackColor();
        unsafe {
            NSDictionary::from_slices::<NSString>(
                &[NSFontAttributeName, NSForegroundColorAttributeName],
                &[font as &AnyObject, &*color as &AnyObject],
            )
        }
    }

    /// 生成菜单栏模板图（2 倍分辨率位图，显示尺寸按点计）
    pub fn mini_widgets(cols: &[(&str, String)]) -> Option<Retained<NSImage>> {
        if cols.is_empty() {
            return None;
        }
        objc2::rc::autoreleasepool(|_| unsafe {
            let label_font = NSFont::systemFontOfSize_weight(7.0, NSFontWeightSemibold);
            let value_font = NSFont::monospacedDigitSystemFontOfSize_weight(11.0, NSFontWeightRegular);
            let la = attrs(&label_font);
            let va = attrs(&value_font);
            // 数值列宽按「100%」固定，数字变化时不抖动
            let value_w = NSString::from_str("100%").sizeWithAttributes(Some(&va)).width;
            let widths: Vec<f64> = cols
                .iter()
                .map(|(l, _)| NSString::from_str(l).sizeWithAttributes(Some(&la)).width.max(value_w).ceil())
                .collect();
            let width_pt = widths.iter().sum::<f64>() + GAP * (cols.len() - 1) as f64;
            let (w, h) = ((width_pt * SCALE).ceil() as usize, (HEIGHT * SCALE) as usize);
            // sRGB 色彩空间是系统缓存的单例，每次取到的是同一个对象
            let cs = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
            // 由 CG 管理像素内存；kCGImageAlphaPremultipliedLast
            let ctx = CGBitmapContextCreate(std::ptr::null_mut(), w, h, 8, 0, cs, 1);
            CGColorSpaceRelease(cs);
            if ctx.is_null() {
                return None;
            }
            CGContextScaleCTM(ctx, SCALE, SCALE);
            let gc = NSGraphicsContext::graphicsContextWithCGContext_flipped(&*(ctx as *const _), false);
            let prev = NSGraphicsContext::currentContext();
            NSGraphicsContext::setCurrentContext(Some(&gc));
            // 坐标原点在左下角；drawAtPoint 的点是文字行框的左下角，基线 = y − descender
            let label_baseline = 11.8;
            let value_baseline = 1.6;
            let mut x = 0.0;
            for ((label, value), cw) in cols.iter().zip(&widths) {
                let ls = NSString::from_str(label);
                let lw = ls.sizeWithAttributes(Some(&la)).width;
                ls.drawAtPoint_withAttributes(
                    NSPoint::new(x + (cw - lw) / 2.0, label_baseline + label_font.descender()),
                    Some(&la),
                );
                let vs = NSString::from_str(value);
                let vw = vs.sizeWithAttributes(Some(&va)).width;
                vs.drawAtPoint_withAttributes(
                    NSPoint::new(x + (cw - vw) / 2.0, value_baseline + value_font.descender()),
                    Some(&va),
                );
                x += cw + GAP;
            }
            gc.flushGraphics();
            NSGraphicsContext::setCurrentContext(prev.as_deref());
            drop(gc);
            let cg = CGBitmapContextCreateImage(ctx);
            CGContextRelease(ctx);
            if cg.is_null() {
                return None;
            }
            let img = NSImage::initWithCGImage_size(
                NSImage::alloc(),
                &*(cg as *const objc2_core_graphics::CGImage),
                NSSize::new(w as f64 / SCALE, HEIGHT),
            );
            CGImageRelease(cg);
            img.setTemplate(true);
            Some(img)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "读取真实系统指标"]
    fn real_sample() {
        let a = sys::cpu_ticks().unwrap();
        std::thread::sleep(Duration::from_millis(500));
        let b = sys::cpu_ticks().unwrap();
        println!("cpu {:?}", sys::cpu_usage(a, b));
        let (u, t) = sys::memory().unwrap();
        println!("mem {} / {} = {:.1}%", gb(u), gb(t), pct(u, t));
        let (u, t) = sys::disk().unwrap();
        println!("disk {} / {} = {:.1}%", gb(u), gb(t), pct(u, t));
        let img = render::mini_widgets(&[("CPU", "12%".into()), ("RAM", "64%".into()), ("SSD", "100%".into())]).unwrap();
        println!("img {:?} template={}", img.size(), img.isTemplate());
    }
}
