// A tick for something the notes say is to be done. The same box in a
// recording's Notes and on its notebook's page: ticking one ticks the other
// (the tick is kept as a hash of the item's text, per recording).

import { CheckIcon } from "./icons";
import "./ItemCheck.css";

export function ItemCheck({ done, label, onToggle }: { done: boolean; label: string; onToggle: (next: boolean) => void }) {
    return (
        <button
            type="button"
            role="checkbox"
            aria-checked={done}
            className={`item-check${done ? " is-done" : ""}`}
            onClick={() => onToggle(!done)}
            aria-label={label}
            title={done ? "Mark as not done" : "Mark as done"}
        >
            {done && <CheckIcon size={12} strokeWidth={3} />}
        </button>
    );
}
