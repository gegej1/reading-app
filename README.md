# 底色（Dise）

「底色」是个人「决策底色」知识库的 Mac App 与网页版。同一套前端有两种形态：

- **Mac App**：基于 Tauri 2（Rust）的桌面应用，直接读写本机的知识库 Git 仓库。
- **网页版**：静态 PWA，计划部署到 GitHub Pages（<https://gegej1.github.io/reading-app/>），通过 GitHub REST API 访问知识库（后续里程碑）。目前网页版显示虚构的演示数据。

> 本仓库公开，只放代码；个人内容在私有知识库里，令牌只存在设备上。测试和演示用的都是虚构数据（`src/domain/__fixtures__/`）。

## 当前进度：M1「读」

五个界面：**今日**（待办计数、最近一次入库、今日一条底色）· **底色**（人物志式的条目卡片，可以盖章确认）· **裁决**（待裁决队列、待抽查清单，只读）· **知识**（判例、概念、人物与公司、体系，带全文搜索和反向链接）· **原料**（逐字原文）。

在 App 里唯一会写入知识库的操作是给底色条目加或去掉标题末尾的 ✅：写入后自动 `git commit`（说明如 `底色：确认 D05`），不会 push。

## 技术栈

Tauri 2 · Vite 6 · React 18 · TypeScript（strict）· Vitest · pnpm · react-router（HashRouter）· react-markdown + remark-gfm · yaml

## 开发命令

环境要求：Node 22+、pnpm 10、Rust 1.90+。macOS 上只需要 Xcode Command Line Tools。

```bash
pnpm install      # 安装依赖
pnpm dev          # 前端开发服务器（演示数据，http://localhost:1420）
pnpm dev:kb       # 前端开发服务器 + 只读读取本机知识库（http://127.0.0.1:1420）
pnpm tauri dev    # 启动桌面应用（开发模式）
pnpm build        # 构建桌面版前端到 dist/（Tauri 打包时使用）
pnpm build:web    # 构建网页版到 dist-web/（base 为 /reading-app/）
pnpm test         # 单元测试
pnpm test:kb      # 只跑真实知识库兼容性测试（仅本机；CI 里自动跳过）
pnpm typecheck    # TypeScript 类型检查
cargo test --manifest-path src-tauri/Cargo.toml   # Rust 测试
```

## 知识库在哪里

桌面 App 和 `pnpm dev:kb` 按这个顺序找知识库：

1. 环境变量 `DISE_KB_PATH`；
2. （仅桌面 App）`~/Library/Application Support/com.gegej1.dise/config.json` 里的 `kbPath`，例如 `{ "kbPath": "/Users/你/Documents/阅读" }`；
3. 默认 `~/Desktop/GHJProject/阅读`。

`pnpm dev:kb` 只读：中间件只提供 `GET /__kb/list` 和 `GET /__kb/read?path=`，只接受本机请求，并且只在开发服务器里存在，不会进入任何构建产物。

## 桌面 App 的行为

- **窗口**：点关闭按钮只是把窗口藏起来，App 仍在运行；点 Dock 图标再显示；⌘Q 退出。再次打开 App 会切回已经在运行的那一个，不会开第二个。
- **外部链接**：http / https 链接用默认浏览器打开（网页版在新标签页打开）。
- **命令行环境**：从 Finder 启动的 App 拿不到 shell 配置里的环境变量，所以 App 启动时在后台跑一次登录 shell（`$SHELL -ilc`，最多 10 秒），记下 `PATH`、代理等变量，之后启动的 `git` 等子进程都用这份环境。失败时退回到固定的 `PATH`（Homebrew、`~/.npm-global/bin` 等）加系统代理设置（`scutil --proxy`）。
- **日志**：`~/Library/Logs/com.gegej1.dise/dise.log`（满 2 MB 轮换，保留 3 份旧文件）。记录每个命令的名称、路径、结果和耗时，以及前端载入结果和未捕获的错误；不记录文件内容、提交说明和环境变量的值。

## 路径安全

前端传来的路径都要过同一套规则（Rust：`src-tauri/src/kb/paths.rs`，TypeScript：`src/store/kbPath.ts`）：只接受知识库内的相对 POSIX 路径；拒绝绝对路径、`.` / `..` 段、`.git/` 和其他隐藏目录、非 `.md` 文件；符号链接解析后必须仍在知识库内。写入是原子的（临时文件 + 重命名），提交只包含这一个文件。

## 目录结构

```text
src/
  domain/       纯领域逻辑：frontmatter、双链、底色、判例、待裁决、入库报告、搜索、loadKb
  store/        KbStore 接口与实现：tauriStore / devStore（只读）/ memoryStore（演示、测试）
  app/          KB 上下文、导航外壳、路由辅助
  features/     各界面：today / dise / rulings / knowledge / raw / page
  ui/           Markdown 渲染（含双链 remark 插件）与通用组件
  styles/       样式（CSS 变量，无组件库）
dev/            `pnpm dev:kb` 的只读中间件（只在开发服务器里使用）
src-tauri/      Tauri 桌面壳（Rust）：kb_root / kb_list / kb_read / kb_write、登录 shell 环境、日志、窗口
runners/        任务提示词模板（后续加入）
```
