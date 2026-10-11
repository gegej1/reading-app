# 底色（Dise）

「底色」是个人「决策底色」知识库的 Mac App 与网页版。同一套前端有两种形态：

- **Mac App**：基于 Tauri 2（Rust）的桌面应用。
- **网页版**：静态 PWA，计划部署到 GitHub Pages（<https://gegej1.github.io/reading-app/>），通过 GitHub REST API 访问知识库。

> 本仓库公开，只放代码；个人内容在私有知识库里，令牌只存在设备上。

目前是脚手架阶段，具体功能在后续里程碑中加入。

## 技术栈

Tauri 2 · Vite 6 · React 18 · TypeScript（strict）· Vitest · pnpm

## 开发命令

环境要求：Node 22+、pnpm 10、Rust 1.90+。macOS 上只需要 Xcode Command Line Tools。

```bash
pnpm install      # 安装依赖
pnpm dev          # 只启动前端开发服务器（http://localhost:1420）
pnpm tauri dev    # 启动桌面应用（开发模式）
pnpm build        # 构建桌面版前端到 dist/（Tauri 打包时使用）
pnpm build:web    # 构建网页版到 dist-web/（base 为 /reading-app/）
pnpm test         # 运行单元测试
pnpm typecheck    # TypeScript 类型检查
```

## 目录结构

```text
src/            前端入口（main.tsx、App.tsx）
  domain/       纯领域逻辑，不依赖 React / Tauri
  store/        应用状态与数据访问
  features/     各功能模块
src-tauri/      Tauri 桌面壳（Rust）
runners/        任务提示词模板（后续加入）
```
