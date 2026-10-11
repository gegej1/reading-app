补充说明（只给 Codex）：AGENTS.md 第九节写的"不能执行命令"，说的是 Claude Code 受限模式里的情况。你在 Codex 的沙箱里只能通过命令读写文件，所以这次允许：
- 用只读命令读取和查找文件：cat、sed、head、tail、ls、find、rg、grep、wc；
- 用 apply_patch 新建或修改文件。
除此之外不要运行任何命令：不运行 git，不删除、不移动、不重命名任何文件（队列文件也一样），不联网。最后一条回复仍然只输出第九节规定的那一行 JSON。
