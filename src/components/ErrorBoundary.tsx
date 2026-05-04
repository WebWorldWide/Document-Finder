import { Component, type ErrorInfo, type ReactNode } from "react";

interface State {
  error: Error | null;
}

interface Props {
  children: ReactNode;
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("ErrorBoundary caught:", error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;

    const message = this.state.error.message || String(this.state.error);
    const stack = this.state.error.stack ?? "";

    return (
      <div className="flex h-screen w-screen items-center justify-center bg-[var(--color-background)] p-8">
        <div className="max-w-2xl rounded-xl border border-rose-500/40 bg-rose-50 p-6 text-[var(--color-foreground)]">
          <h1 className="mb-2 text-lg font-semibold text-rose-700">
            Document Finder hit an error
          </h1>
          <p className="mb-4 text-sm text-[var(--color-muted-foreground)]">
            The app caught a render error before it could go blank. Reload to recover.
          </p>
          <pre className="mb-4 max-h-64 overflow-auto rounded-md border border-[var(--color-border)] bg-[var(--color-card)] p-3 font-mono text-xs">
            {message}
            {stack ? `\n\n${stack}` : ""}
          </pre>
          <button
            type="button"
            onClick={() => window.location.reload()}
            className="rounded-md bg-[var(--color-primary)] px-3 py-1.5 text-sm font-medium text-[var(--color-primary-foreground)]"
          >
            Reload
          </button>
        </div>
      </div>
    );
  }
}
