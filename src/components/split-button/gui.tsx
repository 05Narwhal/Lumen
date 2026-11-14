import { Button, Menu } from "@mantine/core";
import { IconChevronDown } from "@tabler/icons-react";
import type { ReactNode } from "react";
import "./styles.scss";

export interface SplitButtonEntry {
  label: string;
  icon?: ReactNode;
  onClick: () => void;
}

/**
 * A button with a caret that opens the other ways to do the same thing (Round 20 §8): the left
 * part runs the main action, the caret's menu the alternatives. One outline, one height.
 */
export function SplitButton({
  label,
  leftSection,
  onClick,
  menu,
  variant,
}: {
  label: string;
  leftSection?: ReactNode;
  onClick: () => void;
  menu: SplitButtonEntry[];
  variant?: "default" | "filled";
}) {
  return (
    <Button.Group className="split-button">
      <Button variant={variant} leftSection={leftSection} onClick={onClick}>
        {label}
      </Button>
      <Menu position="bottom-end" withinPortal>
        <Menu.Target>
          <Button variant={variant} className="split-button__caret" aria-label="More ways to create">
            <IconChevronDown size={16} />
          </Button>
        </Menu.Target>
        <Menu.Dropdown>
          {menu.map((m) => (
            <Menu.Item key={m.label} leftSection={m.icon} onClick={m.onClick}>
              {m.label}
            </Menu.Item>
          ))}
        </Menu.Dropdown>
      </Menu>
    </Button.Group>
  );
}
