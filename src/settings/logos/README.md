# 服务商 logo

来源：[LobeHub Icons](https://github.com/lobehub/lobe-icons)（npm 包 `@lobehub/icons-static-svg` 1.106.0，MIT 许可），只拷贝了用到的几个 SVG，未作为依赖引入。

文件名对应 `src/settings/pages/engineShared.ts` 里 `AI_PRESETS` 的 `id`，加新服务商时从同一个包里取对应图标（有 `-color` 彩色版优先用彩色版）。

各 logo 的商标权归各自所有者，仅用于在设置界面中标识服务商。

## 已配置供应商的图标

点名称左边的图标可更换：自动 / 通用图标 / 上传图片 / 任一内置 logo，存在该供应商配置的 `icon` 字段里（上传的图片缩成 64px PNG 存 data URL）。

自动模式先按接口地址匹配内置 logo；匹配不到（自定义接口）时由后端 `site_icon.rs` 取接口域名网站的图标（先看首页 `<link rel="icon">`，再试 `/favicon.ico`，并逐级退到上级域名），只访问用户填的域名、不借助第三方图标服务，走该供应商的代理。结果缓存在 `<数据目录>/icons/<域名>`；删除供应商或改接口地址后，没有引擎再用的域名缓存会被清掉。
