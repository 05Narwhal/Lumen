import { motion } from "motion/react";
import { ThemePreview } from "@/components/theme-preview";
import type { PluginThemeMeta } from "@/types/generated/PluginThemeMeta";
import { THEMES, type SchemeChoice, type ThemeChoice } from "@/types/theme";
import "./styles.scss";

export interface ThemePickerValue {
  theme: ThemeChoice;
  scheme: SchemeChoice;
}

const SCHEMES: { value: SchemeChoice; label: string }[] = [
  { value: "light", label: "Light" },
  { value: "dark", label: "Dark" },
  { value: "auto", label: "Auto" },
];

const osScheme = (): "light" | "dark" =>
  typeof window !== "undefined" && window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";

const spring = { type: "spring", stiffness: 420, damping: 34, mass: 0.8 } as const;

/**
 * Four live theme miniatures, then the enabled plugin themes, plus a Light / Dark / Auto control.
 * Shared by setup and Settings. Picking a plugin theme takes its own scheme; picking it again
 * (it is already the theme) leaves the scheme alone.
 */
export function ThemePicker({
  value,
  onChange,
  pluginThemes = [],
}: {
  value: ThemePickerValue;
  onChange: (v: ThemePickerValue) => void;
  pluginThemes?: PluginThemeMeta[];
}) {
  const effective = value.scheme === "auto" ? osScheme() : value.scheme;
  return (
    <div className="theme-picker">
      <div className="theme-picker__grid" role="radiogroup" aria-label="Theme">
        {THEMES.map((t) => {
          const checked = value.theme === t.id;
          return (
            <motion.button
              type="button"
              key={t.id}
              role="radio"
              aria-checked={checked}
              className="theme-picker__card"
              onClick={() => onChange({ ...value, theme: t.id })}
              whileHover={{ y: -3 }}
              whileTap={{ scale: 0.98, y: -1 }}
              transition={spring}
            >
              {checked && <motion.span layoutId="theme-picker-ring" className="theme-picker__ring" transition={spring} />}
              <ThemePreview theme={t.id} scheme={effective} />
              <span className="theme-picker__name">{t.name}</span>
              <span className="theme-picker__tagline">{t.tagline}</span>
            </motion.button>
          );
        })}
        {pluginThemes.map((t) => {
          const checked = typeof value.theme !== "string" && value.theme.plugin === t.plugin && value.theme.theme === t.id;
          const key = `${t.plugin}/${t.id}`;
          return (
            <motion.button
              type="button"
              key={key}
              role="radio"
              aria-checked={checked}
              className="theme-picker__card"
              onClick={() =>
                onChange({
                  theme: { plugin: t.plugin, theme: t.id },
                  scheme: checked ? value.scheme : (t.defaultScheme as SchemeChoice),
                })
              }
              whileHover={{ y: -3 }}
              whileTap={{ scale: 0.98, y: -1 }}
              transition={spring}
            >
              {checked && <motion.span layoutId="theme-picker-ring" className="theme-picker__ring" transition={spring} />}
              <ThemePreview theme={t.base} plugin={key} scheme={effective} />
              <span className="theme-picker__name">{t.name}</span>
              <span className="theme-picker__tagline">{t.tagline}</span>
            </motion.button>
          );
        })}
      </div>
      <div className="theme-picker__scheme" role="radiogroup" aria-label="Colour scheme">
        {SCHEMES.map((s) => {
          const checked = value.scheme === s.value;
          return (
            <button
              type="button"
              key={s.value}
              role="radio"
              aria-checked={checked}
              className="theme-picker__scheme-btn"
              onClick={() => onChange({ ...value, scheme: s.value })}
            >
              {checked && <motion.span layoutId="theme-picker-scheme" className="theme-picker__scheme-pill" transition={spring} />}
              <span className="theme-picker__scheme-label">{s.label}</span>
            </button>
          );
        })}
      </div>
    </div>
  );
}
