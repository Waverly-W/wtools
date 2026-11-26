// 标注图形：数据结构、绘制、命中测试
// 坐标统一用显示器的逻辑坐标（点），绘制时由调用方设置缩放。

export type Tool = "rect" | "ellipse" | "arrow" | "line" | "pen" | "marker" | "mosaic" | "text" | "number";

export interface Pt {
  x: number;
  y: number;
}

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

interface Base {
  id: number;
  color: string;
  /** 线宽 / 字号 / 马赛克块大小 / 序号半径，按图形类型解释 */
  size: number;
}

export type Shape =
  | (Base & { t: "rect" | "ellipse" | "line" | "arrow" | "mosaic"; a: Pt; b: Pt })
  | (Base & { t: "pen" | "marker"; pts: Pt[] })
  | (Base & { t: "text"; a: Pt; text: string; w: number; h: number })
  | (Base & { t: "number"; a: Pt; n: number });

/** 每种工具的三档大小 */
export const SIZES: Record<Tool, [number, number, number]> = {
  rect: [2, 4, 6],
  ellipse: [2, 4, 6],
  arrow: [2, 4, 6],
  line: [2, 4, 6],
  pen: [2, 4, 6],
  marker: [12, 18, 26],
  mosaic: [6, 10, 16],
  text: [14, 18, 26],
  number: [10, 13, 17],
};

export const COLORS = ["#ff3b30", "#ff9500", "#ffcc00", "#34c759", "#007aff", "#af52de", "#000000", "#ffffff"];

export const FONT = '-apple-system, BlinkMacSystemFont, "PingFang SC", sans-serif';
export const LINE_HEIGHT = 1.3;

export function normRect(a: Pt, b: Pt): Rect {
  return { x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) };
}

export interface DrawEnv {
  /** 马赛克：按块大小返回缩小后的整屏图像（每个像素对应一个块），放大绘制即得到像素块效果 */
  mosaic: (block: number) => HTMLCanvasElement;
}

export function draw(ctx: CanvasRenderingContext2D, s: Shape, env: DrawEnv) {
  ctx.save();
  ctx.strokeStyle = s.color;
  ctx.fillStyle = s.color;
  ctx.lineWidth = s.size;
  ctx.lineJoin = "round";
  ctx.lineCap = "round";
  switch (s.t) {
    case "rect": {
      const r = normRect(s.a, s.b);
      ctx.strokeRect(r.x, r.y, r.w, r.h);
      break;
    }
    case "ellipse": {
      const r = normRect(s.a, s.b);
      ctx.beginPath();
      ctx.ellipse(r.x + r.w / 2, r.y + r.h / 2, r.w / 2, r.h / 2, 0, 0, Math.PI * 2);
      ctx.stroke();
      break;
    }
    case "line":
      ctx.beginPath();
      ctx.moveTo(s.a.x, s.a.y);
      ctx.lineTo(s.b.x, s.b.y);
      ctx.stroke();
      break;
    case "arrow":
      drawArrow(ctx, s.a, s.b, s.size);
      break;
    case "pen":
    case "marker":
      if (s.t === "marker") ctx.globalAlpha = 0.4;
      strokePath(ctx, s.pts);
      break;
    case "mosaic": {
      const r = normRect(s.a, s.b);
      ctx.beginPath();
      ctx.rect(r.x, r.y, r.w, r.h);
      ctx.clip();
      ctx.imageSmoothingEnabled = false;
      const m = env.mosaic(s.size);
      ctx.drawImage(m, 0, 0, m.width * s.size, m.height * s.size);
      break;
    }
    case "text":
      ctx.font = `${s.size}px ${FONT}`;
      ctx.textBaseline = "top";
      s.text.split("\n").forEach((line, i) => ctx.fillText(line, s.a.x, s.a.y + i * s.size * LINE_HEIGHT));
      break;
    case "number": {
      const r = s.size;
      ctx.beginPath();
      ctx.arc(s.a.x, s.a.y, r, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillStyle = isLight(s.color) ? "#000" : "#fff";
      ctx.font = `600 ${Math.round(r * 1.1)}px ${FONT}`;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(String(s.n), s.a.x, s.a.y + r * 0.05);
      break;
    }
  }
  ctx.restore();
}

/** 平滑曲线：相邻点的中点之间用二次曲线连接 */
function strokePath(ctx: CanvasRenderingContext2D, pts: Pt[]) {
  if (!pts.length) return;
  ctx.beginPath();
  ctx.moveTo(pts[0].x, pts[0].y);
  if (pts.length === 1) {
    ctx.lineTo(pts[0].x + 0.01, pts[0].y);
  } else {
    for (let i = 1; i < pts.length - 1; i++) {
      const mx = (pts[i].x + pts[i + 1].x) / 2;
      const my = (pts[i].y + pts[i + 1].y) / 2;
      ctx.quadraticCurveTo(pts[i].x, pts[i].y, mx, my);
    }
    const last = pts[pts.length - 1];
    ctx.lineTo(last.x, last.y);
  }
  ctx.stroke();
}

/** 实心箭头：尾部细、箭身渐宽、箭头三角 */
function drawArrow(ctx: CanvasRenderingContext2D, a: Pt, b: Pt, size: number) {
  const len = Math.hypot(b.x - a.x, b.y - a.y);
  if (len < 1) return;
  const head = Math.min(len * 0.6, size * 3 + 10);
  const headW = head * 0.5;
  const shaft = size * 0.8;
  ctx.save();
  ctx.translate(a.x, a.y);
  ctx.rotate(Math.atan2(b.y - a.y, b.x - a.x));
  ctx.beginPath();
  ctx.moveTo(0, -shaft * 0.25);
  ctx.lineTo(len - head, -shaft);
  ctx.lineTo(len - head, -headW);
  ctx.lineTo(len, 0);
  ctx.lineTo(len - head, headW);
  ctx.lineTo(len - head, shaft);
  ctx.lineTo(0, shaft * 0.25);
  ctx.closePath();
  ctx.fill();
  ctx.restore();
}

function isLight(hex: string): boolean {
  const n = parseInt(hex.slice(1), 16);
  const r = n >> 16, g = (n >> 8) & 255, b = n & 255;
  return r * 0.299 + g * 0.587 + b * 0.114 > 186;
}

function distToSegment(p: Pt, a: Pt, b: Pt): number {
  const dx = b.x - a.x, dy = b.y - a.y;
  const l2 = dx * dx + dy * dy;
  const t = l2 ? Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / l2)) : 0;
  return Math.hypot(p.x - (a.x + t * dx), p.y - (a.y + t * dy));
}

/** 图形的外接矩形（用于选中时的虚线框） */
export function bounds(s: Shape): Rect {
  switch (s.t) {
    case "pen":
    case "marker": {
      const xs = s.pts.map((p) => p.x), ys = s.pts.map((p) => p.y);
      const pad = s.size / 2;
      const x = Math.min(...xs) - pad, y = Math.min(...ys) - pad;
      return { x, y, w: Math.max(...xs) + pad - x, h: Math.max(...ys) + pad - y };
    }
    case "text":
      return { x: s.a.x, y: s.a.y, w: s.w, h: s.h };
    case "number":
      return { x: s.a.x - s.size, y: s.a.y - s.size, w: s.size * 2, h: s.size * 2 };
    default:
      return normRect(s.a, s.b);
  }
}

/** 点是否落在图形上（线框类只认边线附近，便于在框内继续画） */
export function hit(s: Shape, p: Pt, tol = 6): boolean {
  switch (s.t) {
    case "rect": {
      const r = normRect(s.a, s.b);
      const t = tol + s.size / 2;
      const inOuter = p.x >= r.x - t && p.x <= r.x + r.w + t && p.y >= r.y - t && p.y <= r.y + r.h + t;
      const inInner = p.x > r.x + t && p.x < r.x + r.w - t && p.y > r.y + t && p.y < r.y + r.h - t;
      return inOuter && !inInner;
    }
    case "ellipse": {
      const r = normRect(s.a, s.b);
      const rx = r.w / 2, ry = r.h / 2;
      if (rx < 1 || ry < 1) return false;
      const d = Math.hypot((p.x - r.x - rx) / rx, (p.y - r.y - ry) / ry);
      return Math.abs(d - 1) * Math.min(rx, ry) <= tol + s.size / 2;
    }
    case "line":
    case "arrow":
      return distToSegment(p, s.a, s.b) <= tol + s.size;
    case "pen":
    case "marker":
      for (let i = 0; i < s.pts.length; i++) {
        if (distToSegment(p, s.pts[i], s.pts[Math.min(i + 1, s.pts.length - 1)]) <= tol + s.size / 2) return true;
      }
      return false;
    case "number":
      return Math.hypot(p.x - s.a.x, p.y - s.a.y) <= s.size + 2;
    default: {
      const r = bounds(s);
      return p.x >= r.x && p.x <= r.x + r.w && p.y >= r.y && p.y <= r.y + r.h;
    }
  }
}

export function move(s: Shape, dx: number, dy: number): Shape {
  const m = (p: Pt) => ({ x: p.x + dx, y: p.y + dy });
  switch (s.t) {
    case "pen":
    case "marker":
      return { ...s, pts: s.pts.map(m) };
    case "text":
    case "number":
      return { ...s, a: m(s.a) };
    default:
      return { ...s, a: m(s.a), b: m(s.b) };
  }
}

/** 图形是否太小（误触），太小的不保留 */
export function degenerate(s: Shape): boolean {
  switch (s.t) {
    case "rect":
    case "ellipse":
    case "mosaic":
      return Math.abs(s.a.x - s.b.x) < 3 || Math.abs(s.a.y - s.b.y) < 3;
    case "line":
    case "arrow":
      return Math.hypot(s.a.x - s.b.x, s.a.y - s.b.y) < 4;
    case "text":
      return !s.text.trim();
    default:
      return false;
  }
}

/** 按住 ⇧：矩形 / 椭圆变成正方形 / 圆，直线 / 箭头吸附到 45° 的倍数 */
export function constrain(t: Tool, a: Pt, b: Pt): Pt {
  const dx = b.x - a.x, dy = b.y - a.y;
  if (t === "rect" || t === "ellipse" || t === "mosaic") {
    const d = Math.max(Math.abs(dx), Math.abs(dy));
    return { x: a.x + Math.sign(dx || 1) * d, y: a.y + Math.sign(dy || 1) * d };
  }
  const ang = Math.round(Math.atan2(dy, dx) / (Math.PI / 4)) * (Math.PI / 4);
  const len = Math.hypot(dx, dy);
  return { x: a.x + Math.cos(ang) * len, y: a.y + Math.sin(ang) * len };
}
