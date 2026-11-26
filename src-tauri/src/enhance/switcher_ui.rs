//! 窗口切换器界面：原生 NSPanel（不占用网页进程）
//!
//! 所有函数只在主线程调用；状态放在线程局部变量里。
//! 外观对齐 DESIGN.md：系统 popover 毛玻璃浮层、圆角 12、1px 边框；
//! 列表内边距 4 + 行圆角 8；行高 40；选中行淡强调色；标题 13 号、应用名 12 号、徽标 11 号。

use super::ax::CGRect;
use super::switcher::Item;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{define_class, msg_send, sel, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSBackingStoreType, NSBox, NSBoxType, NSColor, NSEvent, NSFont,
    NSImage, NSImageScaling, NSImageView, NSLineBreakMode, NSPanel, NSTextField, NSTitlePosition, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSArray, NSPoint, NSRect, NSSize, NSString};
use std::cell::RefCell;
use std::collections::HashMap;

pub const ROW_H: f64 = 40.0;
pub const PANEL_W: f64 = 640.0;
pub const MAX_ROWS: usize = 12;
/// 边框 1 + 列表内边距 4
pub const INSET: f64 = 5.0;
const RADIUS: f64 = 12.0;
const ROW_RADIUS: f64 = 8.0;
const ICON: f64 = 24.0;
const SIDE: f64 = 12.0;
const GAP: f64 = 12.0;
const BADGE_H: f64 = 18.0;
const BADGE_PAD: f64 = 6.0;

struct Row {
    bg: Retained<NSBox>,
    icon: Retained<NSImageView>,
    title: Retained<NSTextField>,
    badge: Retained<NSBox>,
    badge_label: Retained<NSTextField>,
    app: Retained<NSTextField>,
}

struct Ui {
    panel: Retained<NSPanel>,
    effect: Retained<NSVisualEffectView>,
    border: Retained<NSBox>,
    list: Retained<ListView>,
    rows: Vec<Row>,
    items: Vec<Item>,
    selected: usize,
    /// 列表超过 12 行时，第一行显示的序号
    first: usize,
    icons: HashMap<String, Option<Retained<NSImage>>>,
    /// 本次绘制中有新图标：系统异步加载图标，首次绘制只是占位图，稍后需重绘
    icons_loading: bool,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

define_class!(
    /// 承载各行的视图：坐标从上往下，点击某一行即切换到该窗口
    #[unsafe(super(NSView))]
    #[name = "WtoolsSwitcherList"]
    struct ListView;

    impl ListView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// 面板不激活 wtools，第一次点击也要响应
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// 新图标加载完后重新设置一次图片（NSImageView 不会自己刷新）
        #[unsafe(method(refreshIcons))]
        fn refresh_icons(&self) {
            UI.with(|u| {
                let mut u = u.borrow_mut();
                let Some(ui) = u.as_mut() else { return };
                if !ui.panel.isVisible() {
                    return;
                }
                for (i, row) in ui.rows.iter().enumerate() {
                    let Some(item) = ui.items.get(ui.first + i) else { break };
                    let icon = ui.icons.get(&item.path).cloned().flatten();
                    row.icon.setImage(None);
                    row.icon.setImage(icon.as_deref());
                }
            });
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let row = ((p.y - INSET) / ROW_H).floor();
            let index = UI.with(|u| {
                let u = u.borrow();
                let u = u.as_ref()?;
                (row >= 0.0 && (row as usize) < MAX_ROWS)
                    .then(|| u.first + row as usize)
                    .filter(|i| *i < u.items.len())
            });
            if let Some(i) = index {
                super::switcher::pick(i);
            }
        }
    }
);

fn label(mtm: MainThreadMarker, size: f64, color: &NSColor) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    l.setFont(Some(&NSFont::systemFontOfSize(size)));
    l.setTextColor(Some(color));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l.setUsesSingleLineMode(true);
    l
}

fn plain_box(mtm: MainThreadMarker, radius: f64) -> Retained<NSBox> {
    let b = NSBox::new(mtm);
    b.setBoxType(NSBoxType::Custom);
    b.setTitlePosition(NSTitlePosition::NoTitle);
    b.setBorderWidth(0.0);
    b.setCornerRadius(radius);
    b.setContentViewMargins(NSSize::new(0.0, 0.0));
    b.setFillColor(&NSColor::clearColor());
    b
}

impl Ui {
    fn new(mtm: MainThreadMarker) -> Self {
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(PANEL_W, 200.0));
        let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            rect,
            NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            NSBackingStoreType::Buffered,
            false,
        );
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setLevel(101); // NSPopUpMenuWindowLevel
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        panel.setHidesOnDeactivate(false);
        panel.setAnimationBehavior(NSWindowAnimationBehavior::None);
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );

        // 毛玻璃底：与启动器相同的 popover 材质，圆角裁切
        let effect = NSVisualEffectView::new(mtm);
        effect.setMaterial(NSVisualEffectMaterial::Popover);
        effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
        effect.setState(NSVisualEffectState::Active);
        effect.setWantsLayer(true);
        unsafe {
            let layer: *mut AnyObject = msg_send![&*effect, layer];
            if !layer.is_null() {
                let _: () = msg_send![layer, setCornerRadius: RADIUS];
                let _: () = msg_send![layer, setMasksToBounds: true];
            }
        }
        // 1px 边框（NSBox 用动态颜色，浅色 / 深色自动切换）
        let border = plain_box(mtm, RADIUS);
        border.setBorderWidth(1.0);
        border.setBorderColor(&NSColor::separatorColor());

        let list: Retained<ListView> = unsafe { msg_send![ListView::alloc(mtm), initWithFrame: rect] };
        let rows = (0..MAX_ROWS)
            .map(|_| {
                let bg = plain_box(mtm, ROW_RADIUS);
                let icon = NSImageView::new(mtm);
                icon.setImageScaling(NSImageScaling::ScaleProportionallyUpOrDown);
                let title = label(mtm, 13.0, &NSColor::labelColor());
                let app = label(mtm, 12.0, &NSColor::secondaryLabelColor());
                let badge = plain_box(mtm, 4.0);
                let badge_label = label(mtm, 11.0, &NSColor::secondaryLabelColor());
                if let Some(c) = badge.contentView() {
                    c.addSubview(&badge_label);
                }
                if let Some(c) = bg.contentView() {
                    c.addSubview(&icon);
                    c.addSubview(&title);
                    c.addSubview(&badge);
                    c.addSubview(&app);
                }
                bg.setHidden(true);
                list.addSubview(&bg);
                Row { bg, icon, title, badge, badge_label, app }
            })
            .collect();

        effect.addSubview(&border);
        effect.addSubview(&list);
        panel.setContentView(Some(&effect));
        Self { panel, effect, border, list, rows, items: Vec::new(), selected: 0, first: 0, icons: HashMap::new(), icons_loading: false }
    }

    fn is_dark(&self) -> bool {
        let names = NSArray::from_slice(&[unsafe { NSAppearanceNameAqua }, unsafe { NSAppearanceNameDarkAqua }]);
        self.panel
            .effectiveAppearance()
            .bestMatchFromAppearancesWithNames(&names)
            .is_some_and(|n| &*n == unsafe { NSAppearanceNameDarkAqua })
    }

    fn icon(&mut self, path: &str) -> Option<Retained<NSImage>> {
        if path.is_empty() {
            return None;
        }
        self.icons
            .entry(path.to_string())
            .or_insert_with(|| {
                self.icons_loading = true;
                let img = NSWorkspace::sharedWorkspace().iconForFile(&NSString::from_str(path));
                img.setSize(NSSize::new(ICON, ICON));
                Some(img)
            })
            .clone()
    }

    /// 让选中行落在可见范围内
    fn scroll_to_selected(&mut self) {
        if self.selected < self.first {
            self.first = self.selected;
        } else if self.selected >= self.first + MAX_ROWS {
            self.first = self.selected + 1 - MAX_ROWS;
        }
        self.first = self.first.min(self.items.len().saturating_sub(MAX_ROWS));
    }

    /// 有新图标时安排两次延迟刷新
    fn refresh_later(&mut self) {
        if !std::mem::take(&mut self.icons_loading) {
            return;
        }
        for delay in [0.15f64, 0.5] {
            let _: () = unsafe {
                msg_send![&*self.list, performSelector: sel!(refreshIcons), withObject: None::<&AnyObject>, afterDelay: delay]
            };
        }
    }

    fn render(&mut self) {
        // 设计令牌 --selected / --hover 的浅色、深色取值
        let (sel, hover) = if self.is_dark() {
            (NSColor::colorWithSRGBRed_green_blue_alpha(64.0 / 255.0, 140.0 / 255.0, 1.0, 0.28), NSColor::colorWithWhite_alpha(1.0, 0.06))
        } else {
            (NSColor::colorWithSRGBRed_green_blue_alpha(0.0, 102.0 / 255.0, 1.0, 0.14), NSColor::colorWithWhite_alpha(0.0, 0.05))
        };
        let clear = NSColor::clearColor();
        let rw = PANEL_W - INSET * 2.0;
        for i in 0..MAX_ROWS {
            let idx = self.first + i;
            let Some(item) = self.items.get(idx).cloned() else {
                self.rows[i].bg.setHidden(true);
                continue;
            };
            let icon = self.icon(&item.path);
            let row = &self.rows[i];
            row.bg.setHidden(false);
            row.bg.setFrame(NSRect::new(NSPoint::new(INSET, INSET + i as f64 * ROW_H), NSSize::new(rw, ROW_H)));
            row.bg.setFillColor(if idx == self.selected { &sel } else { &clear });

            row.icon.setImage(icon.as_deref());
            row.icon.setFrame(NSRect::new(NSPoint::new(SIDE, (ROW_H - ICON) / 2.0), NSSize::new(ICON, ICON)));

            let center = |h: f64| ((ROW_H - h) / 2.0).round();
            let mut right = rw - SIDE;
            // 应用名：与标题相同时不显示，最多占行宽 30%
            let show_app = item.title != item.app;
            row.app.setHidden(!show_app);
            if show_app {
                row.app.setStringValue(&NSString::from_str(&item.app));
                row.app.sizeToFit();
                let s = row.app.frame().size;
                let w = s.width.ceil().min(rw * 0.3);
                row.app.setFrame(NSRect::new(NSPoint::new(right - w, center(s.height)), NSSize::new(w, s.height)));
                right -= w + GAP;
            }
            // 状态徽标
            let badge = match item.state {
                "minimized" => Some("已最小化"),
                "hidden" => Some("已隐藏"),
                _ => None,
            };
            row.badge.setHidden(badge.is_none());
            if let Some(text) = badge {
                row.badge.setFillColor(&hover);
                row.badge_label.setStringValue(&NSString::from_str(text));
                row.badge_label.sizeToFit();
                let s = row.badge_label.frame().size;
                let w = s.width.ceil() + BADGE_PAD * 2.0;
                row.badge.setFrame(NSRect::new(NSPoint::new(right - w, center(BADGE_H)), NSSize::new(w, BADGE_H)));
                row.badge_label.setFrameOrigin(NSPoint::new(BADGE_PAD, ((BADGE_H - s.height) / 2.0).round()));
                right -= w + GAP;
            }
            row.title.setStringValue(&NSString::from_str(&item.title));
            row.title.sizeToFit();
            let th = row.title.frame().size.height;
            let x = SIDE + ICON + GAP;
            row.title.setFrame(NSRect::new(NSPoint::new(x, center(th)), NSSize::new((right - x).max(0.0), th)));
        }
    }
}

/// 预先创建面板（不显示）
pub fn create(mtm: MainThreadMarker) {
    UI.with(|u| {
        if u.borrow().is_none() {
            *u.borrow_mut() = Some(Ui::new(mtm));
        }
    });
}

pub fn destroy() {
    if let Some(ui) = UI.with(|u| u.borrow_mut().take()) {
        ui.panel.orderOut(None);
        ui.panel.close();
    }
}

/// 显示或更新列表：放到屏幕中央偏上，不激活 wtools（焦点留在原应用）
pub fn present(items: Vec<Item>, selected: usize, screen: CGRect) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    create(mtm);
    UI.with(|u| {
        let mut u = u.borrow_mut();
        let Some(ui) = u.as_mut() else { return };
        ui.items = items;
        ui.selected = selected.min(ui.items.len().saturating_sub(1));
        ui.scroll_to_selected();
        let h = ui.items.len().min(MAX_ROWS) as f64 * ROW_H + INSET * 2.0;
        let x = screen.origin.x + (screen.size.width - PANEL_W) / 2.0;
        let y = screen.origin.y + (screen.size.height - h) / 2.0 + screen.size.height * 0.08;
        let size = NSSize::new(PANEL_W, h);
        ui.panel.setFrame_display(NSRect::new(NSPoint::new(x.round(), y.round()), size), false);
        let bounds = NSRect::new(NSPoint::new(0.0, 0.0), size);
        ui.effect.setFrame(bounds);
        ui.border.setFrame(bounds);
        ui.list.setFrame(bounds);
        ui.render();
        ui.panel.orderFrontRegardless();
        ui.refresh_later();
    });
}

/// 更新选中行（面板未显示时忽略）
pub fn select(selected: usize) {
    UI.with(|u| {
        let mut u = u.borrow_mut();
        let Some(ui) = u.as_mut() else { return };
        if !ui.panel.isVisible() || selected >= ui.items.len() {
            return;
        }
        ui.selected = selected;
        ui.scroll_to_selected();
        ui.render();
        ui.refresh_later();
    });
}

pub fn hide() {
    UI.with(|u| {
        if let Some(ui) = u.borrow_mut().as_mut() {
            ui.panel.orderOut(None);
            ui.items.clear();
        }
    });
}
