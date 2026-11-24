import { Button, createTheme, type CSSVariablesResolver, type PopoverProps } from "@mantine/core";

/**
 * Every dropdown, menu and popover stays inside the window (Round 5). Flip picks the side
 * with more room even when neither fits whole (Mantine's lock otherwise keeps the first
 * side and runs off screen), shift pulls it back in on both axes, and size caps it to the
 * room left, where it scrolls (`globals.scss`).
 */
export const FLOATING_MIDDLEWARES: NonNullable<PopoverProps["middlewares"]> = {
  flip: { fallbackStrategy: "bestFit", padding: 8 },
  shift: { padding: 8, crossAxis: true },
  size: { padding: 8 },
};
const floating = { middlewares: FLOATING_MIDDLEWARES };
const combobox = { comboboxProps: floating };

export const mantineTheme = createTheme({
  fontFamily: "var(--font-body)",
  headings: { fontFamily: "var(--font-heading)", fontWeight: "var(--heading-weight)" },
  defaultRadius: "md",
  primaryShade: 6,
  cursorType: "pointer",
  // Reduced motion is handled by our own tokens (data-reduced-motion), not by Mantine.
  respectReducedMotion: false,
  components: {
    SegmentedControl: { defaultProps: { transitionDuration: 220, transitionTimingFunction: "cubic-bezier(.2,.8,.2,1)" } },
    Switch: { defaultProps: { size: "md" } },
    // Progress takes its colour from Mantine's palette (blue), not the primary-colour
    // variables below; point it at the theme's accent so a bar matches its button.
    Progress: { defaultProps: { color: "var(--accent)" } },
    // Floating surfaces share one entrance, so dialogs, menus and popovers feel related.
    // `data-part` is how a theme's `theme.css` names Lumen's parts (THEMES.md *Parts*). A modal is a
    // `dialog` unless it says otherwise (`confirm`, `palette`, `install-sheet`); a menu's dropdown
    // is a `menu`. Protected parts are never styled by a theme, nor anything inside them.
    Modal: { defaultProps: { "data-part": "dialog", transitionProps: { transition: "pop", duration: 180, timingFunction: "cubic-bezier(.2,.8,.2,1)" }, overlayProps: { blur: 2 } } },
    Popover: { defaultProps: { ...floating, transitionProps: { transition: "pop", duration: 150 } } },
    Menu: { defaultProps: { ...floating, transitionProps: { transition: "pop", duration: 140 } } },
    MenuDropdown: { defaultProps: { "data-part": "menu" } },
    TextInput: { defaultProps: { "data-part": "input" } },
    HoverCard: { defaultProps: floating },
    Combobox: { defaultProps: floating },
    Select: { defaultProps: { ...combobox, allowDeselect: false, checkIconPosition: "right" } },
    MultiSelect: { defaultProps: combobox },
    Autocomplete: { defaultProps: combobox },
    TagsInput: { defaultProps: combobox },
    Tooltip: { defaultProps: { transitionProps: { transition: "fade", duration: 120 } } },
    Drawer: { defaultProps: { transitionProps: { transition: "slide-left", duration: 220 } } },
    // Our accent is a CSS variable, so Mantine cannot derive a contrast colour for filled
    // buttons itself; point its text colour at the theme's accent-fg token.
    Button: Button.extend({
      defaultProps: { radius: "md", "data-part": "button" },
      vars: (_theme, props) =>
        props.variant === undefined || props.variant === "filled"
          ? { root: { "--button-color": "var(--accent-fg)" } }
          : { root: {} },
    }),
  },
});

// Mantine components read these; our SCSS reads the same tokens. They can never disagree.
const shared = {
  "--mantine-color-body": "var(--bg-base)",
  "--mantine-color-text": "var(--fg-primary)",
  "--mantine-color-dimmed": "var(--fg-muted)",
  "--mantine-color-default": "var(--bg-raised)",
  "--mantine-color-default-hover": "var(--bg-sunken)",
  "--mantine-color-default-border": "var(--border)",
  // A disabled control in the theme's own sunken surface and muted text (≥ 3:1 in every
  // theme, contrast.test.ts), not Mantine's grey palette, which ignores ours.
  "--mantine-color-disabled": "var(--bg-sunken)",
  "--mantine-color-disabled-color": "var(--fg-muted)",
  "--mantine-color-disabled-border": "var(--border)",
  "--mantine-primary-color-filled": "var(--accent)",
  "--mantine-primary-color-filled-hover": "var(--accent-hover)",
  "--mantine-primary-color-contrast": "var(--accent-fg)",
  "--mantine-primary-color-light": "var(--accent-soft)",
  "--mantine-primary-color-light-hover": "var(--accent-soft)",
  "--mantine-primary-color-light-color": "var(--accent)",
  // A `light`, `subtle` or `outline` button with no colour reads the primary palette (blue)
  // by name, not the variables above; without these it was Mantine's stock blue everywhere.
  "--mantine-color-blue-filled": "var(--accent)",
  "--mantine-color-blue-filled-hover": "var(--accent-hover)",
  "--mantine-color-blue-light": "var(--accent-soft)",
  "--mantine-color-blue-light-hover": "var(--accent-soft)",
  "--mantine-color-blue-light-color": "var(--accent)",
  "--mantine-color-blue-outline": "var(--accent)",
  "--mantine-color-blue-outline-hover": "var(--accent-soft)",
  "--mantine-color-blue-text": "var(--accent)",
  "--mantine-radius-sm": "var(--radius-sm)",
  "--mantine-radius-md": "var(--radius-md)",
  "--mantine-radius-lg": "var(--radius-lg)",
  "--mantine-radius-default": "var(--radius-md)",
  "--mantine-shadow-sm": "var(--shadow-sm)",
  "--mantine-shadow-md": "var(--shadow-md)",
  "--mantine-shadow-lg": "var(--shadow-lg)",
};

export const cssVariablesResolver: CSSVariablesResolver = () => ({
  variables: {},
  light: shared,
  dark: shared,
});
