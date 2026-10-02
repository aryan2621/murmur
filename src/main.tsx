import { Component, StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { invoke } from "@tauri-apps/api/core";
import "./index.css";
import App from "./App";

// Outside the app (plain browser preview) swap in a mock backend.
if ("__TAURI_INTERNALS__" in window) document.documentElement.classList.add("tauri");
else await import("./dev/mock-tauri");

/** Front-end errors go to murmur.log so they're visible without dev tools. */
const report = (message: string) => invoke("widget_log", { message: `app: ${message}` }).catch(() => {});
window.addEventListener("error", (e) => report(`${e.message} @ ${e.filename}:${e.lineno}`));
window.addEventListener("unhandledrejection", (e) => report(`unhandled: ${String(e.reason)}`));

class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error, info: { componentStack?: string | null }) {
    report(`${error.message}\n${info.componentStack ?? ""}`);
  }
  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="grid h-screen place-items-center p-8 text-center">
        <div>
          <p className="text-sm font-medium">Something went wrong in this page.</p>
          <p className="mt-1 text-xs text-muted-foreground">{this.state.error.message}</p>
          <button className="mt-4 text-sm text-primary underline" onClick={() => location.reload()}>
            Reload
          </button>
        </div>
      </div>
    );
  }
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </StrictMode>,
);
