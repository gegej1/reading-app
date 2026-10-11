import { APP_DISPLAY_NAME } from "./domain";

// Placeholder screen. Real features arrive in later milestones under src/features/.
export default function App() {
  return (
    <main className="app">
      <h1 className="app__name">{APP_DISPLAY_NAME}</h1>
      <p className="app__tagline">
        决策底色知识库的 Mac App 与网页版 · 脚手架已就绪，功能开发中
      </p>
    </main>
  );
}
