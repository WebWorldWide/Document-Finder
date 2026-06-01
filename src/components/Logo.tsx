import type { JSX } from "solid-js";

/**
 * The real Document Finder logo (inlined from icons/Document Finder Icon.svg):
 * a green→blue rounded square holding a white document with wavy text lines and
 * a magnifier. Used in the sidebar brand and the welcome dialog. The gradient id
 * is namespaced so multiple instances don't collide.
 */
export default function Logo(props: { size?: number; class?: string; style?: JSX.CSSProperties }) {
  const size = () => props.size ?? 28;
  return (
    <svg
      width={size()}
      height={size()}
      viewBox="0 0 800 800"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      class={props.class}
      style={props.style}
      aria-hidden="true"
    >
      <rect width="800" height="800" rx="180" fill="url(#df_logo_grad)" />
      <rect x="164" y="79" width="473" height="643" rx="47" fill="white" />
      <path
        d="M233 352.5C263.713 337.785 340.661 317.184 402.753 352.5C464.846 387.816 538.79 367.215 568 352.5"
        stroke="black"
        stroke-width="15"
      />
      <path
        d="M233 544.5C263.713 529.785 340.661 509.184 402.753 544.5C464.846 579.816 538.79 559.215 568 544.5"
        stroke="black"
        stroke-width="15"
      />
      <path
        d="M233 160.5C263.713 145.785 340.661 125.184 402.753 160.5C464.846 195.816 538.79 175.215 568 160.5"
        stroke="black"
        stroke-width="15"
      />
      <path
        d="M233 256.5C263.713 241.785 340.661 221.184 402.753 256.5C464.846 291.816 538.79 271.215 568 256.5"
        stroke="black"
        stroke-width="15"
      />
      <path
        d="M233 640.5C263.713 625.785 340.661 605.184 402.753 640.5C464.846 675.816 538.79 655.215 568 640.5"
        stroke="black"
        stroke-width="15"
      />
      <path
        d="M233 448.5C263.713 433.785 340.661 413.184 402.753 448.5C464.846 483.816 538.79 463.215 568 448.5"
        stroke="black"
        stroke-width="15"
      />
      <circle
        cx="262.026"
        cy="217.452"
        r="133.677"
        transform="rotate(9.18088 262.026 217.452)"
        fill="#B1E6FF"
        fill-opacity="0.36"
        stroke="black"
        stroke-width="25"
      />
      <line
        x1="182.484"
        y1="336.194"
        x2="74.3689"
        y2="494.831"
        stroke="black"
        stroke-width="25"
        stroke-linecap="round"
      />
      <defs>
        <linearGradient
          id="df_logo_grad"
          x1="400"
          y1="0"
          x2="400"
          y2="800"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0.216346" stop-color="#D2E468" />
          <stop offset="0.519231" stop-color="#52C86F" />
          <stop offset="0.880907" stop-color="#6D9FCA" />
        </linearGradient>
      </defs>
    </svg>
  );
}
