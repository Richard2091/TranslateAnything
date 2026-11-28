// 模拟 Tauri 后端的数据与命令，只在界面预览页使用
(() => {
  const now = Math.floor(Date.now() / 1000);
  const config = {
    hotkey: "Alt+T",
    frame_click_through: true,
    frame_corner_radius: 0,
    target_lang: "zh-CN",
    persist_cache: true,
    mode: "ocr",
    ocr_lang: "auto",
    engine: "edge",
    fallback: ["bing"],
    engines: { edge: { base_url: "", api_key: "", region: "", model: "", proxy: null }, openai: { base_url: "https://api.example.com/v1", api_key: "sk-test", region: "", model: "gpt-4o-mini", proxy: null, concurrency: 2 } },
    builtin_glossary: true,
    glossary: { New: "新建", Settings: "设置", Projects: "项目" },
    filter: { keep_terms: ["Claude"], keep_patterns: [], force_translate: [], apps: {} },
    immersive: {
      enabled: true,
      hotkey: "Alt+Shift+T",
      targets: [
        { name: "Claude", enabled: true, exe: "C:\\Users\\Demo\\AppData\\Local\\AnthropicClaude\\claude.exe", args: "", port: 39222, url_prefixes: ["https://claude.ai/"], skip_selectors: [".chat-message"] },
        { name: "Visual Studio Code", enabled: true, exe: "C:\\Program Files\\Microsoft VS Code\\Code.exe", args: "", port: 39223, url_prefixes: [], skip_selectors: [] },
        { name: "Notion", enabled: false, exe: "", args: "", port: 39224, url_prefixes: [], skip_selectors: [] },
      ],
    },
  };
  const statuses = [
    { port: 39222, name: "Claude", enabled: true, state: "attached", message: "", pages: [{ id: "1", url: "https://claude.ai/new", title: "Claude" }, { id: "2", url: "https://claude.ai/code", title: "Claude Code" }], show: true, translated: 1284, engine_batches: 37 },
    { port: 39223, name: "Visual Studio Code", enabled: true, state: "no_port", message: "程序正在运行，但没有带调试端口启动。点「以沉浸模式重启」即可。", pages: [], show: false, translated: 0, engine_batches: 0 },
    { port: 39224, name: "Notion", enabled: false, state: "disabled", message: "", pages: [], show: false, translated: 0, engine_batches: 0 },
  ];
  const words = [["New chat", "新聊天", "claude.exe", "Claude", false], ["Search conversations", "搜索对话", "claude.exe", "Claude", false], ["Recents", "最近", "claude.exe", "Claude", false], ["Start a new project", "开始一个新项目", "", "", false], ["Upload files", "上传文件", "claude.exe", "Claude", false], ["Claude Sonnet 4", "Claude Sonnet 4", "claude.exe", "Claude", true], ["Copy link", "复制链接", "", "", false], ["Rename", "重命名", "claude.exe", "Claude", false], ["Delete conversation", "删除对话", "claude.exe", "Claude", false], ["Model", "模型", "claude.exe", "Claude", false], ["Extended thinking", "扩展思考", "claude.exe", "Claude", false], ["Add to project", "添加到项目", "", "", false]];
  const rows = words.map(([s, t, scope, app_name, keep], i) => ({ lang: "zh-CN", scope, app_name, source: s, target: t, engine: i % 4 === 0 ? "手工" : "edge", manual: i % 4 === 0, keep, hits: 120 - i * 9, used_at: now - i * 4000 }));
  const filterEvents = [
    { text: "Claude Sonnet 4", decision: "keep", reason: "model_version", app_id: "claude.exe", app_name: "Claude", mode: "immersive", at: now - 30 },
    { text: "Step 1", decision: "translate", reason: "force_translate", app_id: "claude.exe", app_name: "Claude", mode: "immersive", at: now - 45 },
    { text: "npm run build", decision: "keep", reason: "code_or_structured", app_id: "code.exe", app_name: "Visual Studio Code", mode: "overlay", at: now - 90 },
  ];
  const builtin = { Cancel: "取消", Close: "关闭", Copy: "复制", Edit: "编辑", Help: "帮助", OK: "确定", Paste: "粘贴", Save: "保存", Undo: "撤销" };
  const catalog = [
    { id: "edge", name: "微软翻译（免费）", needs_key: false, official: false, fields: ["proxy"], note: "Edge 浏览器内置翻译接口，失败自动回退 Bing 网页版；国内可直连" },
    { id: "bing", name: "Bing 网页翻译（免费）", needs_key: false, official: false, fields: ["proxy"], note: "Bing 网页版接口；国内可直连" },
    { id: "google_free", name: "Google 翻译（免费）", needs_key: false, official: false, fields: ["base_url", "proxy"], note: "国内需配置代理；调用频繁会被限流" },
    { id: "microsoft", name: "微软翻译（官方 API）", needs_key: true, official: true, fields: ["api_key", "region", "base_url", "proxy"], note: "Azure Translator 资源，免费层每月 200 万字符" },
    { id: "deepl", name: "DeepL（官方 API）", needs_key: true, official: true, fields: ["api_key", "base_url", "proxy"], note: "免费版每月 50 万字符" },
    { id: "deeplx", name: "DeepLX（自建）", needs_key: false, official: false, fields: ["base_url", "api_key", "proxy"], note: "自建 DeepLX 服务地址" },
    { id: "google", name: "Google Cloud 翻译（官方 API）", needs_key: true, official: true, fields: ["api_key", "base_url", "proxy"], note: "需启用 Cloud Translation API；国内需配置代理" },
    { id: "openai", name: "大模型（OpenAI 兼容）", needs_key: true, official: true, fields: ["base_url", "api_key", "model", "concurrency", "proxy"], note: "任意 OpenAI 兼容服务" },
  ];
  let autostartEnabled = false;
  const handlers = {
    autostart_status: () => autostartEnabled ? "enabled" : "disabled",
    autostart_set: ({ enabled }) => {
      autostartEnabled = enabled;
      return enabled ? "enabled" : "disabled";
    },
    // 界面语言：地址加 ?lang=en 预览英文界面，缺的键按后端规则回退到英文
    ui_i18n: async () => {
      const lang = new URLSearchParams(location.search).get("lang") || "zh-CN";
      const load = async (l) => (await fetch(`/src/i18n/${l}.json`)).json();
      const en = await load("en");
      const own = lang === "en" ? {} : await load(lang);
      return { locale: lang, messages: { ...en, ...own }, locales: [["zh-CN", "简体中文"], ["en", "English"]] };
    },
    settings_get: () => ({ config: JSON.parse(JSON.stringify(config)), config_path: "C:\\Users\\Demo\\.translate-anything\\config.json", db_path: "C:\\Users\\Demo\\.translate-anything\\translations.db", config_size: 1843, db_size: 2457600, db_custom: false, builtin_glossary: builtin }),
    engine_catalog: () => catalog,
    store_stats: () => ({ total: 8421, manual: 37, hits: 152390 }),
    engine_test: async (a) => { await new Promise((r) => setTimeout(r, 600)); if (a.id === "openai") throw "网络错误：连接超时"; return { engine: a.id, source: "Hello! Screen text, translated anywhere.", text: "你好！屏幕上的文字，随处可译。", ms: 238 }; },
    overlay_start: () => null,
    ocr_languages: () => [{ tag: "en-US", name: "英语(美国)" }, { tag: "zh-Hans-CN", name: "中文(简体，中国)" }],
    frames_count: () => 2,
    frames_close_all: () => 2,
    store_pick_and_relocate: () => null,
    store_reset_location: () => "",
    reveal_file: () => null,
    settings_save: (a) => { Object.assign(config, JSON.parse(JSON.stringify(a.config))); },
    immersive_status: () => statuses,
    immersive_detect: () => [{ name: "Slack", exe: "C:\\Users\\Demo\\AppData\\Local\\slack\\slack.exe", port: null, configured: false }, { name: "Claude", exe: "C:\\...\\claude.exe", port: 39222, configured: true }],
    store_apps: () => [
      { id: "claude.exe", name: "Claude", count: rows.filter((x) => x.scope === "claude.exe").length },
      { id: "", name: "", count: rows.filter((x) => x.scope === "").length },
    ],
    store_query: (a) => {
      const k = (a.keyword || "").toLowerCase();
      const r = rows.filter((x) => (a.app === null || a.app === undefined || x.scope === a.app) && (!a.manualOnly || x.manual) && (!k || x.source.toLowerCase().includes(k) || x.target.includes(k)));
      return { total: r.length, rows: r };
    },
    store_set: () => null, store_delete: () => null, store_clear: () => 0,
    filter_events: (a) => filterEvents.filter((e) => (!a.appId || e.app_id === a.appId) && (!a.onlyKept || e.decision === "keep")),
    filter_override: () => null,
    engine_cooldowns: () => [],
    immersive_set_show: () => null, immersive_set_enabled: () => null, immersive_launch: () => "已发送",
  };
  let cb = 0;
  window.__TAURI_INTERNALS__ = {
    transformCallback: () => ++cb,
    unregisterCallback: () => {},
    invoke: async (cmd, args) => {
      if (cmd.startsWith("plugin:event|")) return cb;
      const f = handlers[cmd];
      if (!f) { console.warn("未模拟的命令", cmd, args); return null; }
      return f(args || {});
    },
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
})();
