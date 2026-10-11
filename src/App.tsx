import { HashRouter, Link, Route, Routes } from "react-router";
import { CaptureProvider } from "./app/capture";
import { KbProvider } from "./app/KbProvider";
import { Shell } from "./app/Shell";
import type { KbStore } from "./store/types";
import { DiseScreen } from "./features/dise/DiseScreen";
import { KnowledgeScreen } from "./features/knowledge/KnowledgeScreen";
import { PageScreen } from "./features/page/PageScreen";
import { RawScreen } from "./features/raw/RawScreen";
import { RulingsScreen } from "./features/rulings/RulingsScreen";
import { TodayScreen } from "./features/today/TodayScreen";
import { EmptyState } from "./ui/components";

/** All screens. Exported separately so tests can mount them in a MemoryRouter. */
export function AppRoutes() {
  return (
    <Shell>
      <Routes>
        <Route path="/" element={<TodayScreen />} />
        <Route path="/dise" element={<DiseScreen />} />
        <Route path="/dise/:entryId" element={<DiseScreen />} />
        <Route path="/rulings" element={<RulingsScreen />} />
        <Route path="/knowledge" element={<KnowledgeScreen />} />
        <Route path="/raw" element={<RawScreen />} />
        <Route path="/page/*" element={<PageScreen />} />
        <Route
          path="*"
          element={
            <div className="screen">
              <EmptyState title="这一页不存在">
                <Link to="/">回到今日</Link>
              </EmptyState>
            </div>
          }
        />
      </Routes>
    </Shell>
  );
}

// HashRouter keeps every route inside index.html, which works in the Tauri
// webview and on GitHub Pages without server-side rewrites.
export default function App({ store }: { store?: KbStore }) {
  return (
    <KbProvider store={store}>
      <CaptureProvider>
        <HashRouter>
          <AppRoutes />
        </HashRouter>
      </CaptureProvider>
    </KbProvider>
  );
}
