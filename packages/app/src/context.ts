import { createContext, useContext } from "react";
import type { AppController } from "./controller";

export const ControllerContext = createContext<AppController | null>(null);

export function useController(): AppController {
  const ctl = useContext(ControllerContext);
  if (!ctl) throw new Error("useController outside <App>");
  return ctl;
}

/** Joins class names, skipping falsy values. */
export function cx(...names: (string | false | null | undefined)[]): string {
  return names.filter(Boolean).join(" ");
}
