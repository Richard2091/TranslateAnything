// 设置窗口入口：导航、页面切换、后端事件
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { S, loadStats, loadView, setRerender, type AppVersion, type EngineInfo, type TargetStatus, type UpdateInfo } from "./state";
import { errText, h } from "./ui";
import { setNav } from "./nav";
import { renderDashboard } from "./pages/dashboard";
import { loadFrames, loadOcrLangs, renderOverlay } from "./pages/overlay";
import { renderImmersive } from "./pages/immersive";
import { renderEngines } from "./pages/engines";
import { buildLibrary, queryLibrary } from "./pages/library";
import { glossaryDirty, renderGlossary, resetGlossaryDraft } from "./pages/glossary";
import { renderGeneral } from "./pages/general";
import { initI18n, isZh, t } from "../i18n";

type Page = { render: (root: HTMLElement) => void; onShow?: () => Promise<unknown> | void };

const PAGES: Record<string, Page> = {
  dashboard: { render: renderDashboard, onShow: () => Promise.all([loadFrames(), loadStats()]) },
  overlay: { render: renderOverlay, onShow: () => Promise.all([loadFrames(), loadOcrLangs()]) },
  immersive: { render: renderImmersive },
  engines: { render: renderEngines },
  library: { render: buildLibrary, onShow: queryLibrary },
  glossary: { render: renderGlossary },
  general: { render: renderGeneral },
};

let current = "dashboard";
const rootOf = (id: string) => document.getElementById(`tab-${id}`)!;

/** 只重绘当前页；其他页面切过去时再按最新数据绘制 */
function renderCurrent() {
  pending = false;
  PAGES[current].render(rootOf(current));
}
setRerender(renderCurrent);

/**
 * 后台数据变化（状态推送、定时刷新）引起的重绘：用户正展开下拉框或在输入框里打字时先不重绘，
 * 等下拉框收起、输入框失焦后再补上，免得弹出层被拆掉、输入被打断。
 */
let pending = false;
function backgroundRender() {
  const a = document.activeElement;
  const typing = a instanceof HTMLInputElement || a instanceof HTMLTextAreaElement;
  if (document.querySelector(".dd-pop:not(.leaving)") || (typing && rootOf(current).contains(a))) {
    pending = true;
    return;
  }
  renderCurrent();
}
const flush = () => setTimeout(() => pending && backgroundRender(), 0);
window.addEventListener("dropdown-closed", flush);
document.addEventListener("focusout", flush);

/** 页面入场：卡片依次上浮淡入，播完即撤掉标记 */
const enterTimers = new WeakMap<HTMLElement, number>();
function playEnter(s: HTMLElement) {
  s.classList.remove("entering");
  void s.offsetWidth; // 强制重排，让动画能重新播放
  s.classList.add("entering");
  clearTimeout(enterTimers.get(s));
  enterTimers.set(s, window.setTimeout(() => s.classList.remove("entering"), 800));
}

/** 导航选中条：滑到当前按钮的位置 */
function moveIndicator() {
  const active = document.querySelector<HTMLElement>("#nav button.active");
  const bar = document.querySelector<HTMLElement>(".nav-indicator");
  if (!active || !bar) return;
  // 第一次直接就位，不要从顶部滑下来
  const first = !bar.dataset.placed;
  if (first) bar.style.transition = "none";
  bar.style.transform = `translateY(${active.offsetTop}px)`;
  bar.style.height = `${active.offsetHeight}px`;
  if (first) {
    bar.dataset.placed = "1";
    void bar.offsetWidth;
    bar.style.transition = "";
  }
}

async function switchTab(name: string) {
  if (!PAGES[name]) return;
  current = name;
  for (const b of document.querySelectorAll<HTMLButtonElement>("#nav button")) b.classList.toggle("active", b.dataset.tab === name);
  moveIndicator();
  const root = rootOf(name);
  PAGES[name].render(root);
  for (const s of document.querySelectorAll<HTMLElement>(".tab")) s.classList.toggle("active", s === root);
  document.querySelector("main")!.scrollTop = 0;
  playEnter(root);
  const show = PAGES[name].onShow;
  if (show) {
    await show();
    if (current === name && name !== "library") backgroundRender();
  }
}
setNav((tab) => void switchTab(tab));

for (const b of document.querySelectorAll<HTMLButtonElement>("#nav button")) {
  b.addEventListener("click", () => {
    if (!b.classList.contains("active")) switchTab(b.dataset.tab!);
  });
}

// 滚动：滚过一屏的三分之一显示"回到顶部"
const mainEl = document.querySelector("main")!;
const toTop = document.getElementById("to-top")!;
let scrollTick = 0;
mainEl.addEventListener(
  "scroll",
  () => {
    if (scrollTick) return;
    scrollTick = requestAnimationFrame(() => {
      scrollTick = 0;
      toTop.classList.toggle("show", mainEl.scrollTop > mainEl.clientHeight / 3);
    });
  },
  { passive: true },
);
toTop.addEventListener("click", () => mainEl.scrollTo({ top: 0, behavior: "smooth" }));

// 自绘标题栏的窗口按钮；最大化后按钮换成"向下还原"
const win = getCurrentWindow();
const maxBtn = document.getElementById("win-max")!;
document.getElementById("win-min")!.addEventListener("click", () => void win.minimize());
maxBtn.addEventListener("click", () => void win.toggleMaximize());
document.getElementById("win-close")!.addEventListener("click", () => void win.close());
async function syncMaximized() {
  const max = await win.isMaximized();
  maxBtn.classList.toggle("maximized", max);
  const key = max ? "window.restore" : "window.maximize";
  maxBtn.title = t(key);
  maxBtn.setAttribute("aria-label", t(key));
}
void win.onResized(() => void syncMaximized());

// 概览和悬浮窗页显示"打开中的翻译框"个数，翻译框由用户随手关掉，定时刷新一下
setInterval(async () => {
  if (document.hidden || (current !== "dashboard" && current !== "overlay")) return;
  const before = S.frames;
  await loadFrames();
  if (S.frames !== before) backgroundRender();
}, 2000);

(async () => {
  // 先载入界面文案，再绘制任何内容
  try {
    await initI18n();
  } catch (e) {
    console.error("界面文案载入失败", e);
  }
  // 英文界面下品牌名本身就是 Translate Anything，副标题重复，只在中文界面显示
  const sub = document.querySelector<HTMLElement>(".brand-sub");
  if (sub) sub.hidden = !isZh();
  void syncMaximized();
  try {
    await loadView();
    [S.statuses, S.catalog, S.version, S.update.info] = await Promise.all([
      invoke<TargetStatus[]>("immersive_status"),
      invoke<EngineInfo[]>("engine_catalog"),
      invoke<AppVersion>("app_version"),
      invoke<UpdateInfo | null>("update_state"),
    ]);
    // 从托盘"发现新版本"打开时直接到常规页
    const goto = await invoke<string | null>("settings_take_goto").catch(() => null);
    await switchTab(goto && PAGES[goto] ? goto : "dashboard");
  } catch (e) {
    rootOf("dashboard").replaceChildren(h("div", { class: "note bad" }, t("settings.loadFailed", { error: errText(e) })));
    rootOf("dashboard").classList.add("active");
  }
  moveIndicator();
  await listen("config-changed", async () => {
    await loadView();
    resetGlossaryDraft();
    if (current === "glossary" && glossaryDirty()) return;
    if (current !== "library") backgroundRender();
  });
  await listen<string>("settings-goto", (e) => {
    void invoke("settings_take_goto");
    if (PAGES[e.payload] && current !== e.payload) void switchTab(e.payload);
  });
  await listen<UpdateInfo>("update-state", (e) => {
    S.update.info = e.payload;
    if (current === "general" || current === "dashboard") backgroundRender();
  });
  await listen<number>("update-progress", (e) => {
    S.update.progress = e.payload;
    if (current === "general") backgroundRender();
  });
  await listen<TargetStatus[]>("immersive-status", (e) => {
    S.statuses = e.payload;
    if (current === "dashboard" || current === "immersive") backgroundRender();
  });
})();
