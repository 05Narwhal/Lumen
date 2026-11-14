import { Button } from "@mantine/core";
import { ThemePicker } from "@/components/theme-picker";
import { useSettings } from "@/hooks/use-settings";

export function ThemeStep({ onNext, onBack }: { onNext: () => void; onBack: () => void }) {
  const { settings, update } = useSettings();
  if (!settings) return null;
  return (
    <div className="setup__step">
      <h1 className="setup__title">Pick a look</h1>
      <p className="setup__lead">You can change this any time in Settings.</p>
      <ThemePicker
        value={{ theme: settings.appearance.theme, scheme: settings.appearance.scheme }}
        onChange={(v) =>
          update((s) => ({ ...s, appearance: { ...s.appearance, theme: v.theme, scheme: v.scheme } }))
        }
      />
      <div className="setup__actions">
        <Button variant="subtle" onClick={onBack}>
          Back
        </Button>
        <Button onClick={onNext}>Continue</Button>
      </div>
    </div>
  );
}
