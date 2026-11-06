import { createSignal } from "solid-js";
import type { LibraryInfo } from "@/lib/tauri";

export type View = "discover" | "library" | "settings";

const [view, setView] = createSignal<View>("discover");
/** Library folder to show when the Library view opens. */
const [openLibraryPath, setOpenLibraryPath] = createSignal<string | null>(null);
const [knownLibraries, setKnownLibraries] = createSignal<LibraryInfo[]>([]);
const [listenersReady, setListenersReady] = createSignal(true);
const [announcement, setAnnouncement] = createSignal("");
const [toast, setToast] = createSignal<string | null>(null);

let toastTimer: ReturnType<typeof setTimeout> | undefined;

/** Push text into the always-mounted screen-reader live region. */
function announce(msg: string) {
  setAnnouncement("");
  queueMicrotask(() => setAnnouncement(msg));
}

/** Brief confirmation at the bottom of the window (also announced). */
function showToast(msg: string) {
  setToast(msg);
  announce(msg);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => setToast(null), 3200);
}

export interface ConfirmRequest {
  title: string;
  body: string;
  confirmLabel: string;
  danger?: boolean;
  resolve: (ok: boolean) => void;
}
const [confirmReq, setConfirmReq] = createSignal<ConfirmRequest | null>(null);

/** In-app confirmation dialog (never a blocking browser `confirm`). */
function confirm(opts: Omit<ConfirmRequest, "resolve">): Promise<boolean> {
  return new Promise((resolve) => {
    setConfirmReq({
      ...opts,
      resolve: (ok) => {
        setConfirmReq(null);
        resolve(ok);
      },
    });
  });
}

export const uiStore = {
  get view() {
    return view();
  },
  setView,
  get openLibraryPath() {
    return openLibraryPath();
  },
  /** Navigate to a library in the Library view. */
  showLibrary(path: string | null) {
    setOpenLibraryPath(path);
    setView("library");
  },
  get knownLibraries() {
    return knownLibraries();
  },
  setKnownLibraries,
  get listenersReady() {
    return listenersReady();
  },
  setListenersReady,
  get announcement() {
    return announcement();
  },
  announce,
  get toast() {
    return toast();
  },
  showToast,
  get confirmRequest() {
    return confirmReq();
  },
  confirm,
};
