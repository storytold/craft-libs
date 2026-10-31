/** Types for `react.js` (`@craft/app/react`). */

/** The React face (React >= 18 as a peer, no JSX, no build step). */
export const CraftApp: import('react').ForwardRefExoticComponent<
  import('react').PropsWithoutRef<{
    app: string;
    src: string;
    storage?: unknown | null;
    onReady?: (detail: Record<string, unknown>) => void;
    onFailed?: (detail: Record<string, unknown>) => void;
    onDirty?: (detail: Record<string, unknown>) => void;
    onSave?: (detail: Record<string, unknown>) => void;
    style?: import('react').CSSProperties;
    className?: string;
  }> &
  import('react').RefAttributes<HTMLElement>
>;
