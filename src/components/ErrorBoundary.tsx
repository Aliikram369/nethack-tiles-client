import { Component, type ReactNode } from "react";
import { reportFrontendError } from "../lib/tauri";

interface State {
  error: Error | null;
}

/**
 * Renders the failure instead of an empty window.
 *
 * Without this, any throw during render or in an effect unmounts the tree and
 * the user is left staring at a blank page with nothing to report.
 */
export class ErrorBoundary extends Component<{ children: ReactNode }, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error) {
    void reportFrontendError(`render: ${error.stack ?? error.message}`);
  }

  render() {
    const { error } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="app">
        <header className="masthead">
          <h1>NetHack Tiles</h1>
        </header>
        <p className="banner banner--error" role="alert">
          The interface stopped: {error.message}
        </p>
        <pre className="crash-detail">{error.stack}</pre>
        <div className="form-actions">
          <span className="spacer" />
          <button className="primary" onClick={() => window.location.reload()}>
            Reload
          </button>
        </div>
      </div>
    );
  }
}
