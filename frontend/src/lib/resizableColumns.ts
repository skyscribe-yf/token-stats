import type { MouseEvent as ReactMouseEvent } from "react";
import { useState, useEffect, useRef, useCallback } from "react";

export type ColumnWidths = Record<string, number>;

interface UseResizableColumnsOptions {
  storageKey: string;
  defaultWidths: ColumnWidths;
  minWidth?: number;
}

interface HandleProps {
  onMouseDown: (e: ReactMouseEvent) => void;
  onClick: (e: ReactMouseEvent) => void;
}

export interface UseResizableColumnsResult {
  widths: ColumnWidths;
  resetWidths: () => void;
  getHandleProps: (key: string) => HandleProps;
}

/** Drag-to-resize table columns. Widths are persisted in localStorage. */
export function useResizableColumns({
  storageKey,
  defaultWidths,
  minWidth = 48,
}: UseResizableColumnsOptions): UseResizableColumnsResult {
  const [widths, setWidths] = useState<ColumnWidths>(() => {
    if (typeof window === "undefined") return defaultWidths;
    try {
      const saved = window.localStorage.getItem(storageKey);
      if (saved) {
        const parsed = JSON.parse(saved);
        if (parsed && typeof parsed === "object") {
          return { ...defaultWidths, ...parsed };
        }
      }
    } catch {
      /* ignore */
    }
    return defaultWidths;
  });

  const widthsRef = useRef(widths);
  widthsRef.current = widths;
  const draggingKeyRef = useRef<string | null>(null);
  const startXRef = useRef(0);
  const startWidthRef = useRef(0);
  const activeRef = useRef(false);

  useEffect(() => {
    if (typeof window === "undefined") return;
    try {
      window.localStorage.setItem(storageKey, JSON.stringify(widths));
    } catch {
      /* ignore */
    }
  }, [storageKey, widths]);

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      const key = draggingKeyRef.current;
      if (!key || !activeRef.current) return;
      const delta = e.clientX - startXRef.current;
      const next = Math.max(minWidth, startWidthRef.current + delta);
      setWidths((prev) => ({ ...prev, [key]: next }));
    };
    const onMouseUp = () => {
      if (!activeRef.current) return;
      activeRef.current = false;
      draggingKeyRef.current = null;
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
    };
    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
    return () => {
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
    };
  }, [minWidth]);

  const getHandleProps = useCallback(
    (key: string): HandleProps => ({
      onMouseDown: (e: ReactMouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        draggingKeyRef.current = key;
        startXRef.current = e.clientX;
        startWidthRef.current = widthsRef.current[key] ?? minWidth;
        activeRef.current = true;
        document.body.style.userSelect = "none";
        document.body.style.cursor = "col-resize";
      },
      onClick: (e: ReactMouseEvent) => {
        e.stopPropagation();
      },
    }),
    [minWidth]
  );

  const resetWidths = useCallback(() => {
    setWidths(defaultWidths);
  }, [defaultWidths]);

  return { widths, resetWidths, getHandleProps };
}
