import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { reportFrontendError } from "./lib/tauri";
import "./styles.css";

// Anything that escapes React entirely still needs to reach a log.
window.addEventListener("error", (e) =>
  reportFrontendError(`uncaught: ${e.message} (${e.filename}:${e.lineno})`),
);
window.addEventListener("unhandledrejection", (e) =>
  reportFrontendError(`unhandled rejection: ${e.reason}`),
);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
