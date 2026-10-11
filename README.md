# 底色（Dise）

「底色」是个人「决策底色」知识库的 Mac App 与网页版。同一套前端有两种形态：

- **Mac App**：基于 Tauri 2（Rust）的桌面应用，直接读写本机的知识库 Git 仓库。
- **网页版**：静态 PWA，计划部署到 GitHub Pages（<https://gegej1.github.io/reading-app/>），通过 GitHub REST API 访问知识库（后续里程碑）。目前网页版显示虚构的演示数据。

> 本仓库公开，只放代码；个人内容在私有知识库里，令牌只存在设备上。测试和演示用的都是虚构数据（`src/domain/__fixtures__/`）。

## 现在能做什么

五个界面：**今日**（待办计数、最近一次入库、今日一条底色）· **底色**（人物志式的条目卡片，可以盖章确认）· **裁决**（待裁决队列、待抽查清单，只读）· **知识**（书、判例、概念、人物与公司、体系，带全文搜索和反向链接）· **原料**（逐字原文）。

**记一笔**：随手记下一句话，直接进知识库的收件箱（`inbox/`），等下次入库时整理。桌面 App 里是一张浮在所有窗口之上的小纸条（全局快捷键 ⌃⌥N、菜单栏图标或今日页的「记一笔」），网页版和演示里是页面上的对话框。⌘↩（Windows 上 Ctrl+↩）记下，↩ 换行，Esc 收起但保留草稿；用输入法打字或听写时，候选还没上屏的按键不会触发保存。草稿每改一次都存在本地，App 崩溃也不会丢。

**同步**（桌面 App）：知识库和它的 Git 远端自动保持一致，左侧栏底部显示状态（已同步 / ↑N 待推送 / ↓N 待拉取 / 离线 / 需要处理）。

App 只会以三种方式写入知识库，每次都只提交那一个文件：

- 给底色条目加或去掉标题末尾的 ✅（提交说明如 `底色：确认 D05`）；
- 记一笔：在 `inbox/` 新建 `YYYY-MM-DDTHH-mm-ss-<来源>.md`（来源为 `mac`、`iphone` 或 `web`），YAML 里写 `来源` 和带时区的 `时间`，空一行后是逐字原文（只把 CRLF 换成 LF，结尾留一个换行）；提交说明 `收件箱：记一笔（mac）`。同一秒里的第二条用 `-2`、`-3` 后缀，绝不覆盖已有文件；
- 同步时的 fast-forward / rebase / push（规则见下文「同步」）。

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
pnpm icon         # 由 scripts/build-icon.mjs 重画图标（只用简单图形）并生成 src-tauri/icons/
pnpm icon:tray    # 由 scripts/build-tray-icon.mjs 重画菜单栏图标（单色模板图，只用简单图形）
cargo test --manifest-path src-tauri/Cargo.toml   # Rust 测试
```

## 知识库在哪里

桌面 App 和 `pnpm dev:kb` 按这个顺序找知识库：

1. 环境变量 `DISE_KB_PATH`；
2. （仅桌面 App）`~/Library/Application Support/com.gegej1.dise/config.json` 里的 `kbPath`，例如 `{ "kbPath": "/Users/你/Documents/阅读" }`；
3. 默认 `~/Desktop/GHJProject/阅读`。

`pnpm dev:kb` 只读：中间件只提供 `GET /__kb/list` 和 `GET /__kb/read?path=`，只接受本机请求，并且只在开发服务器里存在，不会进入任何构建产物。

同一个 `config.json` 里还可以用 `captureShortcut` 换掉记一笔的全局快捷键，写法如 `"Ctrl+Alt+K"`、`"Cmd+Shift+Space"`，必须带 ⌃、⌥ 或 ⌘ 之一。默认是 ⌃⌥N（⌘⇧N 已经是 Finder 的「新建文件夹」和 Chrome 的「新的无痕窗口」）。快捷键注册不上（被别的 App 占用，或者写法不对）时 App 照常运行，日志里写明原因，菜单栏和今日页也会提示。

## 桌面 App 的行为

- **窗口**：点关闭按钮只是把窗口藏起来，App 仍在运行；点 Dock 图标再显示；⌘Q 或菜单栏的「退出」退出。再次打开 App 会切回已经在运行的那一个，不会开第二个。
- **菜单栏图标**：一枚单色的印章（模板图，跟随浅色和深色菜单栏）。菜单：记一笔（显示快捷键）/ 打开底色 / 一行状态（收件箱 N 条 · 待裁决 M 项 · 同步状态）/ 立即同步 / 退出。Dock 图标保留。
- **记一笔窗口**：560×300 的无边框小窗口，出现在鼠标所在屏幕的上三分之一处，总在最前，不进 Dock。点到别的 App 时，纸条上没写字就自动收起，写了字就留着。记下或收起后，焦点回到打开它之前的那个 App。这个窗口只能在 `inbox/` 里新建文件，不能读、列出或改写知识库里的任何文件。
- **命令行**：`"/Applications/底色.app/Contents/MacOS/dise" --capture "要记下的话"` 把一条随手记交给正在运行的 App（没有在运行时会启动它，记下后留在菜单栏，不打开主窗口），和在纸条上记下完全一样，结果写进日志。`--sync` 立即同步一次。
- **外部链接**：http / https 链接用默认浏览器打开（网页版在新标签页打开）。
- **命令行环境**：从 Finder 启动的 App 拿不到 shell 配置里的环境变量，所以 App 启动时在后台跑一次登录 shell（`$SHELL -ilc`，最多 10 秒），记下 `PATH`、代理等变量，之后启动的 `git` 等子进程都用这份环境。失败时退回到固定的 `PATH`（Homebrew、`~/.npm-global/bin` 等）加系统代理设置（`scutil --proxy`）。
- **日志**：`~/Library/Logs/com.gegej1.dise/dise.log`（满 2 MB 轮换，保留 3 份旧文件）。记录每个命令的名称、路径、结果和耗时（逐个读取文件只在 debug 级别，每次载入只留一行汇总），每次同步的结果，以及前端载入结果和未捕获的错误；不记录文件内容、随手记的文字、提交说明和环境变量的值。
- **一次只做一件事**：写入、记一笔和同步共用一把「知识库锁」。正在进行的操作结束前，写入最多等 45 秒、同步最多等 20 秒，然后明确报告「知识库正忙」，不会一直卡住。以后的长时间任务也通过这把锁和它们错开。
- **事件**：后端向窗口发送 `kb-changed`（文件被写入、新建或同步时拉到了新内容；主窗口据此重新载入）和 `sync-status`（同步开始、结束，或写入后有新的待推送提交），以后的功能可以直接订阅。

## 同步

数据安全优先于新鲜。每次同步都拿着知识库锁，按这个顺序：

1. 不是仓库根目录、没有上游分支、HEAD 不在分支上，或者有没做完的 rebase / merge / cherry-pick / revert / bisect 时，直接跳过并说明原因。
2. `git fetch` 上游分支，最多 30 秒（另有低速限制）；失败就是「离线」，什么都不改。
3. 只落后：`git merge --ff-only`；如果 git 说会覆盖本地文件，就报告「需要处理」，什么都不改。
4. 分叉：有已跟踪文件改动（含已暂存）时跳过；否则先把 HEAD 存到 `refs/dise/backup/<秒数>`，再 `git rebase`；任何失败都 `git rebase --abort`，确认 HEAD 回到备份那一刻，报告冲突的文件。
5. 领先：`git push <远端> HEAD:<上游分支>`，最多 30 秒；如果远端在这期间有了新提交，再从第 2 步来一次。
6. 备份只保留最新 10 个。

绝不 force push、`reset --hard`、stash、clean、checkout 或 restore 文件，也不碰未跟踪的文件。所有 git 调用都用明确的参数、不经过 shell、使用登录 shell 的环境，并且设置 `GIT_TERMINAL_PROMPT=0`。

什么时候同步：启动后（登录 shell 环境准备好之后）一次；每次写入或记一笔后约 3 秒（连续写入只同步一次）；每 10 分钟一次；主窗口获得焦点时（最多每分钟一次）；菜单栏的「立即同步」、点左侧栏的同步状态，或命令行 `--sync`。同步拉到新内容后，主窗口会重新载入。

## 路径安全

前端传来的路径都要过同一套规则（Rust：`src-tauri/src/kb/paths.rs`，TypeScript：`src/store/kbPath.ts`）：只接受知识库内的相对 POSIX 路径；拒绝绝对路径、`.` / `..` 段、`.git/` 和其他隐藏目录、非 `.md` 文件；符号链接解析后必须仍在知识库内。改写是原子的（临时文件 + 重命名），新建也是（临时文件 + 硬链接，名字已被占用时失败而不是覆盖；提交失败时撤回新文件）。提交只包含这一个文件。

## 目录结构

```text
src/
  domain/       纯领域逻辑：frontmatter、双链、底色、判例、待裁决、入库报告、搜索、记一笔、loadKb
  store/        KbStore 接口与实现：tauriStore / devStore（只读）/ memoryStore（演示、测试）
  app/          KB 上下文、导航外壳、路由辅助、记一笔入口、同步状态
  features/     各界面：today / dise / rulings / knowledge / raw / page，以及记一笔的纸条（capture）
  capture/      记一笔窗口的页面（capture.html）
  ui/           Markdown 渲染（含双链 remark 插件）与通用组件
  styles/       样式（CSS 变量，无组件库）
dev/            `pnpm dev:kb` 的只读中间件（只在开发服务器里使用）
src-tauri/      Tauri 桌面壳（Rust）：知识库命令、知识库锁、同步、记一笔窗口、全局快捷键、菜单栏图标、命令行、登录 shell 环境、日志、窗口
runners/        任务提示词模板（后续加入）
```
