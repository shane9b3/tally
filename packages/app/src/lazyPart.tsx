/**
 * Code-split UI that cannot take the app down (see `lazyChunks.ts`).
 *
 * - `lazyPart(load)`: a `React.lazy` component whose failed `import()` is not
 *   cached: after a failure has been shown, the next mount loads it again
 *   (React.lazy alone keeps the rejected promise forever).
 * - `<LazySlot onFail>`: its own `<Suspense fallback={null}>` (one per part, so
 *   a part that is still loading never hides a sibling that is already shown)
 *   inside an error boundary. A failed load shows a toast, runs `onFail` (close
 *   the dialog / reset its store flag) and renders nothing; the grid and the
 *   rest of the chrome stay alive.
 */
import { Component, createElement, lazy, Suspense, type ComponentType, type ErrorInfo, type ReactNode } from "react";
import { t } from "./i18n";
import { useUiStore } from "./stores/uiStore";

/** Parts whose last load failed; `retryFailedParts` gives them a fresh `React.lazy`. */
const failed = new Set<() => void>();

/** Lets every part that failed to load try again on its next mount. */
export function retryFailedParts(): void {
  const resets = [...failed];
  failed.clear();
  for (const reset of resets) reset();
}

export type LazyPart<P extends object> = ComponentType<P>;

export function lazyPart<P extends object>(load: () => Promise<ComponentType<P>>): LazyPart<P> {
  const make = () =>
    lazy(() =>
      load().then(
        (c) => ({ default: c }),
        (err: unknown) => {
          // Swapped only once the boundary has shown the failure (not right away, or
          // React's re-render after the rejection would retry in a loop).
          failed.add(reset);
          throw err;
        },
      ),
    );
  let Lazy = make();
  const reset = (): void => {
    Lazy = make();
  };
  function Part(props: P) {
    return createElement(Lazy as ComponentType<P>, props);
  }
  return Part;
}

interface SlotProps {
  children?: ReactNode;
  /** Close whatever was opening (store flag, dialog); called once per failure. */
  onFail?: () => void;
}

/** One lazy part: its own Suspense boundary inside an error boundary. */
export class LazySlot extends Component<SlotProps, { error: boolean }> {
  override state = { error: false };

  static getDerivedStateFromError(): { error: boolean } {
    return { error: true };
  }

  override componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error("Tally: a part of the UI failed to load", error, info.componentStack);
    useUiStore.getState().pushToast("error", t("app.loadFailed", "This part of Tally couldn't be loaded. Try again."));
    // The next mount of the part loads it again; this slot stays empty until its parent
    // unmounts it (closing the dialog below), so a lasting failure cannot loop.
    retryFailedParts();
    this.props.onFail?.();
  }

  override render(): ReactNode {
    if (this.state.error) return null;
    return createElement(Suspense, { fallback: null }, this.props.children);
  }
}
