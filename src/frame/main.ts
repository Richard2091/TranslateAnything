// 实时翻译框：按后端推送的版面，在原文位置盖上色块并显示译文（沉浸式效果）
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { initFrameCornerRadius } from "./appearance";
import { ScrollFollower } from "./scroll";
import { initI18n, t } from "../i18n";

interface Block {
  /** 物理像素，相对翻译框左上角 */
  x: number;
  y: number;
  w: number;
  h: number;
  lines: number;
  line_h: number;
  /** 原文字号（em，物理像素） */
  font: number;
  /** 原文行距（物理像素），单行块为 0 */
  pitch: number;
  /** 多行块首行上留白（物理像素） */
  pad_top: number;
  weight: number;
  align: "left" | "center";
  bg: string;
  fg: string;
  source: string;
  text: string;
  marker?: { text: string; indent: number; offset: number };
  /** 钉在屏幕上：原地没动的块（固定的标题栏、侧栏、字幕），x / y 为当前位置，不随滚动平移 */
  pinned?: boolean;
}

interface Layout {
  /** 几何变化计数 */
  epoch: number;
  /** 版面坐标对应的累计滚动量、当前累计滚动量（物理像素） */
  base: [number, number];
  total: [number, number];
  blocks: Block[];
  pending: number;
  error: string | null;
  engine: string | null;
}

interface Shift {
  epoch: number;
  total: [number, number];
  /** 这帧画面截于发出前多少毫秒 */
  age: number;
  /** 发出时刻（Unix 毫秒） */
  sent: number;
}

/** 一个块最终采用的排版参数（CSS 像素） */
interface Fit {
  size: number;
  lineHeight: number;
  padTop: number;
}

type ResizeDir =
  | "North" | "South" | "West" | "East"
  | "NorthWest" | "NorthEast" | "SouthWest" | "SouthEast";

const win = getCurrentWindow();
const blocksEl = document.getElementById("blocks")!;
const pinnedEl = document.getElementById("pinned")!;
const statusEl = document.getElementById("status")!;
const errorEl = document.getElementById("error")!;
const modeStatusEl = document.getElementById("mode-status")!;
const btnMode = document.getElementById("btn-mode")!;
const btnPause = document.getElementById("btn-pause")!;
const btnOriginal = document.getElementById("btn-original")!;

/** 最小字号（CSS 像素） */
const MIN_FONT = 9;
/** 放不下时收紧到的行距倍数 */
const TIGHT_LINE = 1.2;

let paused = false;
let showOriginal = false;
/** 交互模式只以本窗口后端返回的值为准；未读到时不启用整框拖动。 */
let clickThrough: boolean | null = null;
let modePending = false;
let interactionError: string | null = null;
let modeChangedAt = 0;
let modeNoticeTimer: number | undefined;
/** 当前版面所属的几何变化计数、基准滚动量、最新累计滚动量（物理像素） */
let epoch = -1;
/** 翻译框移动/缩放后，丢弃计数小于该值的消息 */
let minEpoch = 0;
let base: [number, number] = [0, 0];
/** 滚动跟随：实测位移 + 滚轮预测，逐帧给出译文层该平移到哪 */
const follower = new ScrollFollower();
let rafId = 0;
/** 排版结果缓存：滚动时同一段反复重绘，不必每次都量一遍 */
const fitCache = new Map<string, Fit>();

function css(v: number) {
  return v / window.devicePixelRatio;
}

function px(v: number) {
  return `${css(v)}px`;
}

function overflows(el: HTMLElement) {
  return el.scrollHeight > el.clientHeight + 1 || el.scrollWidth > el.clientWidth + 1;
}

function applyFit(el: HTMLElement, f: Fit) {
  el.style.fontSize = `${f.size}px`;
  el.style.lineHeight = `${f.lineHeight}px`;
  el.style.paddingTop = `${f.padTop}px`;
}

/**
 * 译文与原文同字号、同行距：单行块让文字在块内垂直居中，多行块首行对齐原文首行。
 * 放不下时先把行距收紧，再逐步缩小字号。
 */
function fitText(el: HTMLElement, b: Block) {
  const key = JSON.stringify([b.text, b.w, b.h, b.font, b.pitch, b.pad_top, b.weight, b.marker, window.devicePixelRatio]);
  const cached = fitCache.get(key);
  if (cached) {
    applyFit(el, cached);
    return;
  }
  let size = Math.max(MIN_FONT, css(b.font));
  const f: Fit = {
    size,
    lineHeight: b.pitch > 0 ? css(b.pitch) : css(b.h),
    padTop: b.pitch > 0 ? css(b.pad_top) : 0,
  };
  applyFit(el, f);
  if (overflows(el)) {
    f.padTop = 0;
    f.lineHeight = Math.min(f.lineHeight, size * TIGHT_LINE);
    applyFit(el, f);
    while (size > MIN_FONT && overflows(el)) {
      size = Math.max(MIN_FONT, size * 0.94);
      f.size = size;
      f.lineHeight = size * TIGHT_LINE;
      applyFit(el, f);
    }
  }
  if (fitCache.size > 2000) fitCache.clear();
  fitCache.set(key, { ...f });
}

/** 按显示用的滚动量平移整个译文层；跟随不确定时变淡 */
function applyOffset() {
  const [x, y] = follower.display;
  blocksEl.style.transform = `translate3d(${px(x - base[0])}, ${px(y - base[1])}, 0)`;
  blocksEl.style.opacity = follower.opacity < 1 ? follower.opacity.toFixed(2) : "";
}

function frameTick(now: number) {
  follower.scale = window.devicePixelRatio;
  follower.tick(now);
  applyOffset();
  rafId = follower.active ? requestAnimationFrame(frameTick) : 0;
}

/** 有新的位移或滚轮输入时启动逐帧刷新（已在跑就不重复启动） */
function kick() {
  if (!rafId) rafId = requestAnimationFrame(frameTick);
}

/** 最近一次版面：文案载入完成后据此补上状态提示 */
let lastLayout: Layout | null = null;

function updateStatus(layout: Layout) {
  statusEl.className = layout.error ? "err" : layout.pending > 0 ? "busy" : "";
  statusEl.title = [
    layout.engine ? t("frame.engine", { name: layout.engine }) : "",
    layout.pending > 0 ? t("frame.pending", { n: layout.pending }) : "",
  ]
    .filter(Boolean)
    .join("\n");
}

function render(layout: Layout) {
  if (layout.epoch < minEpoch) return;
  if (layout.epoch !== epoch) follower.reset(layout.total);
  else follower.sync(layout.total, performance.now());
  epoch = layout.epoch;
  base = layout.base;
  applyOffset();
  kick();
  const frag = document.createDocumentFragment();
  const pinnedFrag = document.createDocumentFragment();
  const toFit: [HTMLElement, Block][] = [];
  for (const b of layout.blocks) {
    const el = document.createElement("div");
    el.className = "block";
    Object.assign(el.style, {
      left: px(b.x),
      top: px(b.y),
      width: px(b.w),
      height: px(b.h),
      background: b.bg,
      color: b.fg,
      fontWeight: String(b.weight),
      textAlign: b.align,
    });
    if (b.marker) {
      el.classList.add("block-list");
      el.style.gridTemplateColumns = `${px(b.marker.indent)} minmax(0, 1fr)`;
      const marker = document.createElement("span");
      marker.className = "block-marker";
      marker.style.paddingLeft = px(b.marker.offset);
      marker.textContent = b.marker.text;
      const body = document.createElement("span");
      body.textContent = b.text;
      el.append(marker, body);
    } else {
      el.textContent = b.text;
    }
    (b.pinned ? pinnedFrag : frag).appendChild(el);
    toFit.push([el, b]);
  }
  blocksEl.replaceChildren(frag);
  pinnedEl.replaceChildren(pinnedFrag);
  for (const [el, b] of toFit) fitText(el, b);

  lastLayout = layout;
  updateStatus(layout);
  errorEl.textContent = layout.error ?? "";
  errorEl.hidden = !layout.error;
}

const layoutListener = listen<Layout>("layout", (e) => render(e.payload));

function updateModeUi() {
  const dragMode = clickThrough === false;
  document.body.classList.toggle("drag-mode", dragMode);
  document.body.classList.toggle("interaction-error", interactionError !== null);
  btnMode.classList.toggle("on", clickThrough === true);
  btnMode.setAttribute("aria-disabled", String(modePending));
  if (clickThrough !== null) btnMode.setAttribute("aria-pressed", String(clickThrough));

  const showNotice = modeNoticeTimer !== undefined;
  const modeStatus = !showNotice || clickThrough === null ? "" : t(dragMode ? "frame.drag_on" : "frame.click_through_on");
  const error = interactionError === null ? "" : t("frame.interaction_error", { e: interactionError });
  document.body.classList.toggle("mode-notice", showNotice);
  btnMode.title = error || (clickThrough === null
    ? t("frame.click_through")
    : t(dragMode ? "frame.switch_to_click_through" : "frame.switch_to_drag"));
  modeStatusEl.textContent = error || modeStatus;
}

async function changeMode() {
  if (modePending) return;
  modePending = true;
  updateModeUi();
  try {
    // 未读到初始值时重试读取，不猜测原生窗口的模式。
    const next = clickThrough === null
      ? await invoke<boolean>("frame_get_click_through")
      : await invoke<boolean>("frame_set_click_through", { clickThrough: !clickThrough });
    if (clickThrough !== null && next !== clickThrough) {
      modeChangedAt = Date.now();
      window.clearTimeout(modeNoticeTimer);
      modeNoticeTimer = window.setTimeout(() => {
        modeNoticeTimer = undefined;
        updateModeUi();
      }, 1400);
      // 切换命中范围时，取消尚未结束的滚轮预测并对齐最近的实测位置。
      if (follower.active) {
        follower.reset(follower.measured);
        applyOffset();
      }
    }
    clickThrough = next;
    interactionError = null;
  } catch (error) {
    interactionError = String(error);
  } finally {
    modePending = false;
    updateModeUi();
  }
}

function updateTranslationControls() {
  const pauseLabel = t(paused ? "frame.resume" : "frame.pause");
  btnPause.title = pauseLabel;
  btnPause.setAttribute("aria-label", pauseLabel);
  btnPause.classList.toggle("on", paused);
  const originalLabel = t(showOriginal ? "frame.show_translation" : "frame.show_original");
  btnOriginal.title = originalLabel;
  btnOriginal.setAttribute("aria-label", originalLabel);
  btnOriginal.classList.toggle("on", showOriginal);
}

/** 边框线宽（物理像素），与框选遮罩的选区边框一致 */
const BORDER_PX = 2;
const visualEl = document.getElementById("frame-visual")!;
let borderRaf = 0;
let viewportSize: { width: number; height: number } | undefined;

/** 窗口变化频繁时，每帧只按最后一次的物理像素尺寸设置边框与裁剪层。 */
function fitBorder(size: { width: number; height: number }) {
  viewportSize = size;
  if (borderRaf) return;
  borderRaf = requestAnimationFrame(() => {
    borderRaf = 0;
    const { width, height } = viewportSize!;
    // WebView 视口实际尺寸可能略大于客户区，取较小者，防止右边和下边被裁掉。
    visualEl.style.width = `min(100%, ${px(width)})`;
    visualEl.style.height = `min(100%, ${px(height)})`;
    visualEl.style.setProperty("--frame-stroke", px(BORDER_PX));
  });
}
const borderSize = win.innerSize().then((size) => {
  if (!viewportSize) fitBorder(size);
});


/** 事件从后端发出到页面收到花了多久（毫秒）：两边用同一个系统时钟 */
function transit(sent: number) {
  return Math.min(500, Math.max(0, Date.now() - sent));
}

// 内容滚动：OCR 结果出来之前先整体平移现有译文
listen<Shift>("shift", (e) => {
  if (e.payload.epoch < minEpoch || e.payload.epoch !== epoch) return;
  follower.onMeasured(e.payload.total, performance.now(), e.payload.age + transit(e.payload.sent));
  kick();
});

// 框内滚轮：先按学到的滚动量预测平移，不等截屏比对
listen<{ epoch: number; delta: number; sent: number }>("wheel", (e) => {
  if (clickThrough !== true || modePending || e.payload.epoch < minEpoch || e.payload.epoch !== epoch || e.payload.sent < modeChangedAt) return;
  follower.onWheel(e.payload.delta, performance.now() - transit(e.payload.sent));
  kick();
});

// 由后端轮询悬停位置，穿透时页面自己收不到鼠标进入事件。
const hoverListener = listen<string>("hover", (e) => {
  document.body.classList.toggle("hover", e.payload !== "outside");
});

// 移动或缩放时旧译文位置已失效，先清掉，等新版面
function onGeometryChange() {
  // 同一轮移动/缩放只清理一次，拒收旧范围的布局与位移。
  if (minEpoch > epoch) return;
  minEpoch = epoch + 1;
  cancelAnimationFrame(rafId);
  rafId = 0;
  blocksEl.replaceChildren();
  pinnedEl.replaceChildren();
}
const geometryListeners = Promise.all([
  win.onMoved(onGeometryChange),
  win.onResized(({ payload }) => {
    fitBorder(payload);
    onGeometryChange();
  }),
  win.onScaleChanged(({ payload }) => {
    fitBorder(payload.size);
    onGeometryChange();
  }),
]);

// 单独处理边缘和抓手，工具条内其余区域不会触发整框拖动。
document.body.addEventListener("mousedown", (e) => {
  if (e.button !== 0 || !(e.target instanceof Element)) return;
  const edge = e.target.closest<HTMLElement>(".edge");
  if (edge) {
    e.preventDefault();
    void win.startResizeDragging(edge.dataset.dir as ResizeDir).catch(console.error);
  } else if (e.target.closest("#btn-move") ||
    (clickThrough === false && !modePending && !e.target.closest("#toolbar"))) {
    e.preventDefault();
    void win.startDragging().catch(console.error);
  }
});

btnMode.addEventListener("click", changeMode);

btnPause.addEventListener("click", () => {
  paused = !paused;
  document.body.classList.toggle("paused", paused);
  updateTranslationControls();
  invoke("frame_set_paused", { paused });
});

btnOriginal.addEventListener("click", () => {
  showOriginal = !showOriginal;
  blocksEl.classList.toggle("hidden", showOriginal);
  pinnedEl.classList.toggle("hidden", showOriginal);
  updateTranslationControls();
});

document.getElementById("btn-close")!.addEventListener("click", () => win.close());

// 语言切换会重载页面。每次都按后端当前模式初始化，再报告首帧以关闭框选遮罩。
async function init() {
  await Promise.allSettled([
    initI18n(), borderSize, layoutListener, hoverListener, geometryListeners, changeMode(), initFrameCornerRadius(visualEl),
  ]);
  updateModeUi();
  updateTranslationControls();
  if (lastLayout) updateStatus(lastLayout);
  requestAnimationFrame(() => requestAnimationFrame(() => {
    void invoke("frame_ready").catch(console.error);
  }));
}

void init();
