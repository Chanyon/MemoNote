# 白板笔记（Whiteboard Notes）

一款使用 Rust 与 [GPUI Kit](https://crates.io/crates/gpui-kit) 构建的本地无限画布笔记应用。笔记直接保存为 Markdown 或纯文本文件，白板布局保存在同名的 `.board.json` 中，适合整理灵感、阅读笔记和思维导图。

## 功能

- **无限画布**：自由平移、缩放，自动适应内容。
- **本地笔记库**：按目录管理 Markdown / 纯文本笔记，支持新建、重命名和删除。
- **文本卡片**：支持 Markdown 与纯文本渲染，可拖拽、缩放和旋转。
- **思维导图**：添加根节点与子节点，拖拽子树，自动避让重叠；长主题自动换行并调整节点高度。
- **图片卡片**：从剪贴板粘贴图片，随笔记一起保存在本地。
- **多选与布局**：`Shift` 框选思维导图节点，支持批量拖动。
- **界面定制**：简体中文 / English 切换、亮色 / 暗色模式、预设主题和自定义配色。
- **本地优先**：不依赖账号或云端，所有数据保存在本机。

## 运行

环境要求：Rust 1.85+（使用 Rust 2024 edition），当前主要面向 Windows 10/11。

```powershell
# 开发运行
cargo run

# 构建 release 版本
cargo build --release
```

Release 可执行文件位于 `target/release/whiteboard-notes.exe`。Release 构建会隐藏 CMD 控制台和原生 Windows 标题栏，使用应用内自绘标题栏；Debug 构建则保留控制台和原生标题栏，便于调试。

## 快捷键

| 快捷键 | 功能 |
| --- | --- |
| `Ctrl+N` | 新建笔记 |
| `Ctrl+Shift+N` | 新建目录 |
| `Ctrl+E` | 编辑选中的卡片 |
| `Ctrl+V` | 粘贴剪贴板图片 |
| `Delete` / `Backspace` | 删除选中节点 |
| `+` / `-` | 放大 / 缩小 |
| `0` | 适应画布内容 |
| `Esc` | 取消选择或关闭弹窗 |

鼠标操作：拖动画布空白处平移，滚轮平移，`Ctrl+滚轮` 缩放，双击卡片编辑，右键节点打开菜单，`Shift+拖拽` 框选思维导图节点。

## 数据与配置

- 笔记默认保存在 `文档\WhiteboardNotes`，可在设置中修改保存目录。
- 每篇笔记对应同名的 `.board.json` 文件，粘贴的图片位于 `.assets` 目录。
- 应用配置保存在 `%APPDATA%\WhiteboardNotes\settings.json`。
- 调试或自动化测试时可使用环境变量：
  - `WHITEBOARD_HOME`：覆盖默认笔记目录。
  - `WHITEBOARD_CONFIG`：覆盖配置文件路径。

## 技术栈

- **Rust**：应用逻辑与文件存储。
- **GPUI Kit**：桌面 UI、主题和交互组件。
- **Serde / serde_json**：白板与配置序列化。
- **image / arboard**：图片处理与剪贴板。
- **uuid**：节点标识。

## 项目结构

```text
src/
  main.rs      应用启动、窗口选项与主题初始化
  app.rs       界面、画布交互、思维导图、编辑器与文件树
  model.rs     笔记数据模型、白板持久化与文件操作
  settings.rs  配置、语言、主题预设与保存路径
```

## 开发检查

```powershell
cargo fmt --check
cargo clippy --all-targets
cargo test
```
