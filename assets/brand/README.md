# 枢光 · LumaGate

名称：LumaGate；中文名：枢光。图形使用两条相向的 L 形路径，表示入口、出口和路由通道。为本项目原创 SVG，不依赖在线图标服务。

- 应用图标母版：`lumagate-app.svg`，1024×1024，透明外边缘，深色底与白 / 浅蓝图形。
- 界面 / favicon：`src/manual/assets/lumagate-mark.svg`，针对 24–32px 光学校正。
- 菜单栏模板：`lumagate-tray.svg`，纯黑图形，透明背景；是否显示菜单栏由应用功能决定，本轮不启用新菜单栏功能。

生成应用资源：

```bash
pnpm tauri icon assets/brand/lumagate-app.svg
pnpm tauri icon assets/brand/lumagate-tray.svg --png 24 --png 48 --png 72 -o /tmp/lumagate-tray
```

模板图分别对应 `src-tauri/icons/tray/macos/statusTemplate.png`、`statusTemplate@2x.png`、`statusbar_template_3x.png`。

应用标识为 `cn.restry.lumagate`，数据目录为 `~/.lumagate`，可执行文件为 `lumagate`。旧客户端 RPC / profile 与路由标识保持兼容；数据迁移规则见根目录 README。
