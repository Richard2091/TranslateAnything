<p align="center">
  <img src="design/logo/icon.svg" width="112" alt="Translate Anything">
</p>

<h1 align="center">Translate Anything</h1>

<p align="center"><b>Translate any text on your screen, right where it is.</b></p>

<p align="center"><a href="README.md">简体中文</a> · <b>English</b></p>

<p align="center">
  <a href="https://github.com/Richard2091/TranslateAnything/releases/latest"><img src="https://img.shields.io/github/v/release/Richard2091/TranslateAnything?label=download&color=3d7bff" alt="Download"></a>
  <img src="https://img.shields.io/badge/platform-Windows%2010%20%2F%2011-27c6ec" alt="Platform">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-a3adc2" alt="License"></a>
</p>

<p align="center">
  <img src="docs/images/settings-dashboard.png" width="820" alt="Translate Anything settings: overview">
</p>

Translate Anything turns foreign-language text on your screen into your own language: app interfaces are rewritten in place, and anything else — images, videos, scans — gets a translation overlay on top. No copy-pasting, no switching windows.

The interface follows your system language (Chinese and English for now, falling back to English), and you can change it under **General → Display language**.

## Two ways to translate

### Overlay mode: select it, read it

Press <kbd>Alt</kbd> + <kbd>T</kbd> and drag over any part of the screen. Translate Anything recognizes the text and lays the translation over the original positions, so it looks as if the app were in your language.

- **Live**: translations refresh as the content scrolls or changes. You can move and resize frames and adjust their corner radius in settings
- **Click-through**: switch each frame between click-through and drag mode. In click-through mode you can keep using the app underneath
- **Offline recognition**: text is recognized with the OCR engine built into Windows; screenshots never leave your PC
- **Works on anything**: images, videos, scanned documents, remote desktops — if it's on screen, it can be translated

### Immersive mode: the app itself speaks your language

For supported apps, Translate Anything replaces interface text with translations inside the app, without modifying any of the app's files. Press <kbd>Alt</kbd> + <kbd>Shift</kbd> + <kbd>T</kbd> to switch between original and translation at any time.

- Supports some desktop apps built on **Electron, WebView2, or CEF**
- Detects running apps and relaunches them in immersive mode with one click. With automatic relaunch enabled, Translate Anything tries to switch the app to immersive mode just after it opens. If the app stays open, you can restart it manually
- Identity check: only programs you added are taken over; if a different program shows up behind the port, takeover is refused
- Input boxes, code blocks and similar areas are skipped automatically, and you can exclude more (e.g. chat messages) yourself

<table>
  <tr>
    <td><img src="docs/images/settings-overlay.png" alt="Overlay mode settings"></td>
    <td><img src="docs/images/settings-immersive.png" alt="Immersive mode settings"></td>
  </tr>
  <tr>
    <td align="center">Overlay mode</td>
    <td align="center">Immersive mode</td>
  </tr>
</table>

<sub>Screenshots show the Chinese interface.</sub>

## Pick your translation engine

Works out of the box — **no account or API key needed**. The free Microsoft Translator endpoint is used by default, with automatic fallback to other engines if it fails.

| Engine | Key required | Notes |
|---|:---:|---|
| Microsoft Translator (free) | — | Default |
| Bing web translator (free) | — | |
| Google Translate (free) | — | May need a proxy in some regions |
| Microsoft Translator (official API) | ✓ | 2M characters/month free tier |
| DeepL (official API) | ✓ | 500K characters/month on the free plan |
| Google Cloud Translation | ✓ | Official API |
| DeepLX | Optional | Self-hosted |
| **LLM** | ✓ | Any OpenAI-compatible service, for more natural translations |

On the **Engines** page you can switch between **translation engines / LLM** with one click, enable or disable engines with a switch and drag them into order — they are called from top to bottom, moving on to the next if one fails. Each engine is tested before it can be enabled, and you can test the speed of any engine at any time.

<p align="center">
  <img src="docs/images/settings-engines.png" width="820" alt="Engine settings">
</p>

## Gets faster the more you use it

- **Translation memory**: translated text is stored locally and reused next time without going online. You can search and edit it; edited translations take effect immediately and are never overwritten
- **Glossary**: pin how certain terms are translated (e.g. New → 新建); a built-in list of common interface terms is included
- The translation memory can be moved to another folder at any time, and its file moves with it

## Download

Get the latest version from [Releases](https://github.com/Richard2091/TranslateAnything/releases/latest):

| Your PC | Download |
|---|---|
| Most PCs (Intel / AMD) | the `.exe` with `x64` in its name (`.msi` for managed deployment) |
| ARM-based PCs (e.g. Snapdragon) | the `.exe` with `arm64` in its name |
| No install / run from a USB drive | the `_portable.zip` for your architecture; unzip and run `TranslateAnything.exe` |

The portable version keeps settings and translation memory in the `data` folder next to the program and writes nothing to your user folder. Delete the `portable` file next to it to use the same user folder as the installed version.

Requirements: Windows 10 / 11. The portable version requires the WebView2 runtime. The installers are not code-signed yet, so Windows may show "Windows protected your PC" on first run — click **More info → Run anyway**.

## Quick start

1. After installing, Translate Anything lives in the system tray; left-click the tray icon to open settings
2. **Overlay translation**: press <kbd>Alt</kbd> + <kbd>T</kbd>, drag over an area, and the translation appears
3. **Immersive mode**: on the Immersive mode page, click **Detect running apps**, pick the app to translate, then click **Launch in immersive mode**
4. To switch engines or connect an LLM, go to the Engines page; changes take effect immediately
5. In the installed version, turn on **Launch at startup** under **Settings → General** to start in the system tray when you sign in to Windows

## Privacy

- Text recognition runs entirely on your PC; screenshots are never uploaded
- Recognized text is sent to the translation service you choose to get translations; Translate Anything itself collects and uploads nothing
- Settings and translation memory are stored locally in `~/.translate-anything/` (portable version: the `data` folder next to the program)
- Immersive mode makes the target app temporarily open a local debugging port (bound to this PC only). Turn off automatic relaunch or immersive translation for that app, then restart it normally to close the port

## Roadmap

- Translation filter: automatically skip code, paths, brand names and other text that shouldn't be translated
- Translation scope: translate all, an allowlist or a blocklist per app; press a hotkey to pick areas right inside the app
- Visual translation: LLMs translating directly from the image
- Live speech interpretation, speech-to-subtitles
- More interface languages
- macOS / Linux versions

## Contributing

Tech stack: Tauri 2 + Rust + TypeScript.

```bash
pnpm install
pnpm tauri dev
```

Project documentation is written in Chinese:

| Document | Contents |
|---|---|
| [docs/01-需求说明.md](docs/01-需求说明.md) | Features, interaction flows, acceptance criteria |
| [docs/02-技术方案.md](docs/02-技术方案.md) | Architecture, modules, config fields, i18n, packaging |
| [docs/03-开发环境.md](docs/03-开发环境.md) | Toolchain setup and verification |
| [design/logo/](design/logo/) | Logo design notes and generator script |

Interface text lives in `src/i18n/<language>.json` (shared by the Rust backend and the front end). To add a language, add a JSON file and one line to `LOCALES` in `src-tauri/src/i18n.rs`; missing keys fall back to English.

To tweak the settings UI without running the whole app: run `pnpm dev` and open `http://localhost:1420/design/settings-preview/index.html` to preview it in a browser with mock data (add `?lang=en` for the English interface).

## License

[MIT](LICENSE)
