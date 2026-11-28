<p align="center">
  <img src="design/logo/icon.svg" width="112" alt="随译">
</p>

<h1 align="center">随译 Translate Anything</h1>

<p align="center"><b>屏幕上的文字，随处可译。</b></p>

<p align="center"><b>简体中文</b> · <a href="README.en.md">English</a></p>

<p align="center">
  <a href="https://github.com/Richard2091/TranslateAnything/releases/latest"><img src="https://img.shields.io/github/v/release/Richard2091/TranslateAnything?label=%E4%B8%8B%E8%BD%BD&color=3d7bff" alt="下载"></a>
  <img src="https://img.shields.io/badge/%E5%B9%B3%E5%8F%B0-Windows%2010%20%2F%2011-27c6ec" alt="平台">
  <a href="LICENSE"><img src="https://img.shields.io/badge/%E8%AE%B8%E5%8F%AF%E8%AF%81-MIT-a3adc2" alt="许可证"></a>
</p>

<p align="center">
  <img src="docs/images/settings-dashboard.png" width="820" alt="随译设置窗口：概览">
</p>

随译把屏幕上看到的外文直接变成中文：软件界面在原处改写，图片、视频等任意画面用浮窗盖上译文。不用复制粘贴，也不用切换窗口。

界面默认跟随系统语言，目前支持简体中文和英文，也可以在「常规」里更改。

## 两种翻译方式

### 悬浮窗模式：框哪里，译哪里

按 <kbd>Alt</kbd> + <kbd>T</kbd> 框选屏幕上的任意区域，随译识别其中的文字，把译文按原来的位置盖上去，看起来就像软件本身就是中文的。

- **实时跟随**：内容滚动或变化时译文自动刷新，翻译框可以拖动、缩放，圆角可在设置中调整
- **不挡操作**：每个翻译框都能切换鼠标穿透和框内拖动，穿透时可照常点击下面的软件
- **离线识别**：文字识别用 Windows 自带的引擎，截图不出本机
- **随处可用**：图片、视频、扫描件、远程桌面……只要屏幕上看得见就能翻

### 沉浸模式：软件界面原地变中文

对支持的应用，随译直接在应用内部把界面文字换成译文，不修改软件本身的任何文件。按 <kbd>Alt</kbd> + <kbd>Shift</kbd> + <kbd>T</kbd> 随时在原文和译文之间切换。

- 支持部分基于 **Electron、WebView2 或 CEF** 的桌面应用
- 可识别正在运行的应用，一键重启进入沉浸模式。开启自动重启后，随译会尝试在目标应用刚打开时将它切到沉浸模式。若应用未正常退出，可手动重启
- 身份校验：只接管你添加过的程序，端口背后换了程序会拒绝接管
- 输入框、代码区等不该翻的地方自动跳过，聊天正文之类的区域也可以自己指定不翻

<!-- 沉浸模式效果图：请用没有登录个人账号、没有私人内容的干净客户端截取，放到 docs/images/immersive-demo.png 后在这里引用 -->

<table>
  <tr>
    <td><img src="docs/images/settings-overlay.png" alt="悬浮窗模式设置"></td>
    <td><img src="docs/images/settings-immersive.png" alt="沉浸模式设置"></td>
  </tr>
  <tr>
    <td align="center">悬浮窗模式</td>
    <td align="center">沉浸模式</td>
  </tr>
</table>

## 翻译引擎，按需选择

开箱即用，**不需要注册任何账号或填写密钥**。默认使用免费的微软翻译，失败时自动换用备选引擎。

| 引擎 | 需要密钥 | 说明 |
|---|:---:|---|
| 微软翻译（免费） | — | 默认引擎，国内可直连 |
| Bing 网页翻译（免费） | — | 国内可直连 |
| Google 翻译（免费） | — | 国内需配置代理 |
| 微软翻译（官方接口） | ✓ | 免费额度每月 200 万字符 |
| DeepL（官方接口） | ✓ | 免费版每月 50 万字符 |
| Google Cloud 翻译 | ✓ | 官方接口 |
| DeepLX | 可选 | 自建服务 |
| **大模型** | ✓ | 任意 OpenAI 兼容服务，译文更自然 |

在"翻译引擎"页面一键切换**翻译引擎 / 大模型**，引擎用开关启用 / 停用、拖动排序，按从上到下的顺序调用（前一个出错自动换下一个）；启用前会先测试，通过了才生效，每个引擎也都可以一键测速。

<p align="center">
  <img src="docs/images/settings-engines.png" width="820" alt="翻译引擎设置">
</p>

## 越用越快

- **译文库**：翻译过的文字存在本地，再次遇到直接使用，不再联网请求。可以搜索、改译，改过的译文立即生效且不会被覆盖
- **术语表**：固定某些词的译法（例如 New → 新建）或保留原文，可按软件设置，内置一份常见界面用语
- 译文库位置可以随时更改，文件会一并搬过去

## 下载安装

到 [Releases](https://github.com/Richard2091/TranslateAnything/releases/latest) 下载最新版本：

| 你的电脑 | 下载 |
|---|---|
| 大多数电脑（Intel / AMD 处理器） | 文件名带 `x64` 的 `.exe`（企业批量部署可选 `.msi`） |
| 骁龙等 ARM 处理器的电脑 | 文件名带 `arm64` 的 `.exe` |
| 不想安装 / 放 U 盘随身带 | 对应架构的 `_portable.zip`，解压后运行 `TranslateAnything.exe` |

便携版的设置和译文库保存在解压目录下的 `data` 文件夹，不写用户目录；删掉同目录的 `portable` 文件就会改用和安装版一样的用户目录。

系统要求：Windows 10 / 11，便携版需要系统安装 WebView2 运行时。安装包暂未做代码签名，首次运行时 Windows 可能提示"已保护你的电脑"，点"更多信息 → 仍要运行"即可。

## 快速上手

1. 安装后随译常驻在系统托盘，左键单击托盘图标打开设置窗口
2. **框选翻译**：按 <kbd>Alt</kbd> + <kbd>T</kbd>，拖出一个区域，译文就出现了
3. **沉浸模式**：在"沉浸模式"页点"识别正在运行的应用"，选中要翻译的软件，再点"以沉浸模式启动"
4. 想换翻译引擎或接入大模型，到"翻译引擎"页设置；改完立即生效
5. 安装版可在「设置 → 常规」打开**开机自启动**。登录 Windows 后，随译会驻留系统托盘

## 隐私

- 文字识别完全在本机完成，截图不会上传
- 识别出的文字会发送给你选择的翻译服务，用来获取译文；随译本身不收集、不上传任何数据
- 设置、译文库都保存在本机的 `~/.translate-anything/` 目录（便携版在程序旁的 `data` 文件夹）
- 沉浸模式会让目标应用临时开启本机调试端口（只监听本机）。关闭该应用的自动重启或接管开关后，正常重启该应用即可关闭端口

## 规划中

- 翻译过滤：自动跳过代码、路径、品牌名等不该翻的内容
- 翻译范围：按应用选择全部 / 白名单 / 黑名单，按快捷键直接在应用里点选要翻或不翻的区域
- 视觉直译：大模型直接看图翻译
- 音频同声传译、语音转字幕
- 更多界面语言（目前为简体中文和英文，界面语言默认跟随系统）
- macOS / Linux 版本

## 参与开发

技术栈：Tauri 2 + Rust + TypeScript。

```bash
pnpm install
pnpm tauri dev
```

| 文档 | 内容 |
|---|---|
| [docs/01-需求说明.md](docs/01-需求说明.md) | 功能清单、交互流程、验收标准 |
| [docs/02-技术方案.md](docs/02-技术方案.md) | 架构、模块划分、配置字段、多平台打包 |
| [docs/03-开发环境.md](docs/03-开发环境.md) | 工具链安装与验证 |
| [design/logo/](design/logo/) | Logo 设计说明与生成脚本 |

调整设置界面时可以不启动整个应用：`pnpm dev` 后打开 `http://localhost:1420/design/settings-preview/index.html`，用模拟数据在浏览器里预览（地址加 `?lang=en` 预览英文界面）。

## 许可证

[MIT](LICENSE)
