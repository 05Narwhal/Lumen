import { Button } from "@mantine/core";

export function WelcomeStep({ onNext }: { onNext: () => void }) {
  return (
    <div className="setup__step setup__step--center">
      <div className="setup__wordmark">Lumen</div>
      <p className="setup__lead">Your photos, on your machine, fast.</p>
      <Button size="md" onClick={onNext}>
        Get started
      </Button>
    </div>
  );
}
