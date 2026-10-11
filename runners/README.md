# runners

「底色」发起 AI 运行时交给引擎（Claude Code 或 Codex）的任务提示词。文件内容在编译时嵌进 App（`src-tauri/src/run/prompt.rs`），所以改了这里要重新构建。

| 文件 | 任务 | 队列 |
|---|---|---|
| `ingest.md` | 入库 | `inbox/` 根目录下的 `*.md` |
| `land-rulings.md` | 落裁决 | `inbox/裁决/` 下的 `*.md` |
| `codex.md` | 两个任务都用，只给 Codex | 接在任务提示词后面 |

App 会替换两个占位符：

- `{{date}}`：运行开始时的本地日期，`YYYY-MM-DD`；
- `{{queue_files}}`：这次要处理的队列文件，一行一个，写成 `- inbox/文件名.md`。

`codex.md` 只在用 Codex 时接在后面：知识库 `AGENTS.md` 第九节的"不能执行命令"写的是 Claude Code 受限模式，而 Codex 只能通过命令读文件，不说明的话它会什么都不读、什么都不做。补充说明允许只读命令和 apply_patch，其余命令（git、删除、移动）照样不许。

规则本身不写在这里：提示词只说"做哪个任务、处理哪些文件"，具体怎么做由知识库自己的 `AGENTS.md` 决定（第二节入库、第三节落裁决、第九节由 App 发起的运行）。

只放通用模板，不放任何个人内容或令牌。
