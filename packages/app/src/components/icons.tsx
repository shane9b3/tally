/** Small stroke icons (16 px, currentColor). */
import type { SVGProps } from "react";

function Svg(props: SVGProps<SVGSVGElement>) {
  return (
    <svg
      width={16}
      height={16}
      viewBox="0 0 16 16"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...props}
    />
  );
}

export const UndoIcon = () => (
  <Svg>
    <path d="M5.5 4 2.5 7l3 3" />
    <path d="M2.5 7h7a3.5 3.5 0 0 1 0 7H8" />
  </Svg>
);

export const RedoIcon = () => (
  <Svg>
    <path d="m10.5 4 3 3-3 3" />
    <path d="M13.5 7h-7a3.5 3.5 0 0 0 0 7H8" />
  </Svg>
);

export const SortIcon = () => (
  <Svg>
    <path d="M5 2.5v11M2.5 11 5 13.5 7.5 11" />
    <path d="M11 13.5v-11M8.5 5 11 2.5 13.5 5" />
  </Svg>
);

export const FilterIcon = () => (
  <Svg>
    <path d="M2.5 3.5h11L9.25 8.5v4l-2.5 1.25V8.5z" />
  </Svg>
);

export const ChartIcon = () => (
  <Svg>
    <path d="M3 13.5V8M8 13.5V3M13 13.5V6" strokeWidth={2.2} />
  </Svg>
);

export const PlusIcon = () => (
  <Svg>
    <path d="M8 3v10M3 8h10" />
  </Svg>
);

export const ChevronIcon = () => (
  <Svg width={12} height={12}>
    <path d="m4.5 6 3.5 3.5L11.5 6" />
  </Svg>
);

export const SidebarIcon = () => (
  <Svg>
    <rect x="2" y="3" width="12" height="10" rx="2" />
    <path d="M6 3v10" />
  </Svg>
);

export const SparkIcon = () => (
  <Svg>
    <path d="M8 2v3M8 11v3M2 8h3M11 8h3M4 4l1.8 1.8M10.2 10.2 12 12M12 4l-1.8 1.8M5.8 10.2 4 12" />
  </Svg>
);

export const AlignLeftIcon = () => (
  <Svg>
    <path d="M2.5 4h11M2.5 7h7M2.5 10h11M2.5 13h7" />
  </Svg>
);

export const AlignCenterIcon = () => (
  <Svg>
    <path d="M2.5 4h11M4.5 7h7M2.5 10h11M4.5 13h7" />
  </Svg>
);

export const AlignRightIcon = () => (
  <Svg>
    <path d="M2.5 4h11M6.5 7h7M2.5 10h11M6.5 13h7" />
  </Svg>
);

export const WrapIcon = () => (
  <Svg>
    <path d="M2.5 4h11M2.5 8h9a2 2 0 0 1 0 4H8.5" />
    <path d="m10 10.5-1.5 1.5 1.5 1.5M2.5 12h3" />
  </Svg>
);

export const FillIcon = () => (
  <Svg>
    <path d="M7 2.5 12.5 8 8 12.5a1.4 1.4 0 0 1-2 0L2.5 9a1.4 1.4 0 0 1 0-2z" />
    <path d="M3 8h9" />
    <path d="M13.5 10.5s1 1.3 1 2a1 1 0 0 1-2 0c0-.7 1-2 1-2z" fill="currentColor" stroke="none" />
  </Svg>
);

export const GlanceIcon = () => (
  <Svg>
    <rect x="2" y="2.5" width="12" height="11" rx="2" />
    <path d="M10 2.5v11M11.8 5.5h0M11.8 8h0" />
  </Svg>
);

export const ChipsIcon = () => (
  <Svg>
    <rect x="1.5" y="3" width="8" height="4" rx="2" />
    <rect x="6.5" y="9" width="8" height="4" rx="2" />
  </Svg>
);

export const HighlightIcon = () => (
  <Svg>
    <rect x="1.5" y="2.5" width="13" height="11" rx="2" />
    <path d="M1.5 6.5h13M1.5 10h13M6 2.5v11" />
    <rect x="6.6" y="7.1" width="7.3" height="2.3" fill="currentColor" stroke="none" opacity="0.55" />
  </Svg>
);

export const KeyboardIcon = () => (
  <Svg>
    <rect x="1.5" y="4" width="13" height="8" rx="1.5" />
    <path d="M4 6.5h0M6.5 6.5h0M9 6.5h0M11.5 6.5h0M5 9.5h6" />
  </Svg>
);

export const MoonIcon = () => (
  <Svg>
    <path d="M13 9.5A5.5 5.5 0 0 1 6.5 3a5.5 5.5 0 1 0 6.5 6.5z" />
  </Svg>
);

export const ArrowUpIcon = () => (
  <Svg>
    <path d="M8 13V3M4 7l4-4 4 4" />
  </Svg>
);

export const ArrowDownIcon = () => (
  <Svg>
    <path d="M8 3v10M4 9l4 4 4-4" />
  </Svg>
);

export const MergeIcon = () => (
  <Svg>
    <rect x="2" y="3.5" width="12" height="9" rx="1.5" />
    <path d="M4.5 8h7M6 6l-1.5 2L6 10M10 6l1.5 2L10 10" />
  </Svg>
);

/** Borders: a grid square with the outline drawn heavier. */
export const BordersIcon = () => (
  <Svg>
    <rect x="2.5" y="2.5" width="11" height="11" rx="1" strokeWidth={1.75} />
    <path d="M8 2.5v11M2.5 8h11" strokeWidth={1} strokeDasharray="1.5 1.5" />
  </Svg>
);

export const SearchIcon = () => (
  <Svg>
    <circle cx="7" cy="7" r="4.25" />
    <path d="m10.25 10.25 3.25 3.25" />
  </Svg>
);

/** A small ✕ (12 px), e.g. a document tab's close button. */
export const CloseSmallIcon = () => (
  <Svg width={12} height={12} viewBox="0 0 12 12" strokeWidth={1.4}>
    <path d="M3 3l6 6M9 3l-6 6" />
  </Svg>
);
