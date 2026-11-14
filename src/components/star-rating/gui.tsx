import { IconStar, IconStarFilled } from "@tabler/icons-react";
import "./styles.scss";

/**
 * Five stars. Clicking the star that is already the rating clears it, which is what every
 * photo tool does and what users expect from a rating row.
 */
export function StarRating({
  value,
  onChange,
  size = 16,
  readOnly = false,
}: {
  value: number;
  onChange?: (rating: number) => void;
  size?: number;
  readOnly?: boolean;
}) {
  return (
    <div className="star-rating" role={readOnly ? "img" : "group"} aria-label={`Rating: ${value} of 5`}>
      {[1, 2, 3, 4, 5].map((n) => {
        const filled = n <= value;
        if (readOnly) {
          return (
            <span key={n} className="star-rating__star" data-filled={filled} aria-hidden>
              {filled ? <IconStarFilled size={size} /> : <IconStar size={size} />}
            </span>
          );
        }
        return (
          <button
            key={n}
            type="button"
            className="star-rating__star"
            data-filled={filled}
            aria-label={n === value ? "Clear rating" : `Rate ${n} star${n === 1 ? "" : "s"}`}
            aria-pressed={filled}
            onClick={() => onChange?.(n === value ? 0 : n)}
          >
            {filled ? <IconStarFilled size={size} /> : <IconStar size={size} />}
          </button>
        );
      })}
    </div>
  );
}
