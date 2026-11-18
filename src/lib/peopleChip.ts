// The people chip in a recording's title row (People.tsx): who was in the
// calendar event, in a few words. The list itself opens from the chip, so
// the recording keeps the page.

export interface ChipPerson {
    name: string | null;
    email: string;
}

/** What to call someone in a short list: their first name, or the name part of their address. */
export function shortName(p: ChipPerson): string {
    const name = (p.name ?? "").trim();
    if (name) return name.split(/\s+/)[0];
    return p.email.split("@")[0] || p.email;
}

/** "Maya Chen", "Maya and Jordan", "Maya, Jordan +8". Empty when nobody else was invited. */
export function chipLabel(people: ChipPerson[]): string {
    if (people.length === 0) return "";
    if (people.length === 1) return (people[0].name ?? "").trim() || shortName(people[0]);
    const [a, b] = people.map(shortName);
    if (people.length === 2) return `${a} and ${b}`;
    return `${a}, ${b} +${people.length - 2}`;
}

/** What the chip says to a screen reader, and on hover. */
export function chipTitle(people: ChipPerson[], event: string | null): string {
    const where = event ? ` in "${event}"` : " in the calendar event";
    if (people.length === 0) return `The calendar event${event ? ` "${event}"` : ""}`;
    return `${people.length} ${people.length === 1 ? "person" : "people"}${where}. Show them.`;
}

/**
 * Where the list opens, so it stays inside the recording's pane: `left` is
 * how far to move it from the chip's left edge (0 or negative), `maxHeight`
 * what fits under the chip. All in pixels.
 */
export function popoverPlace(chip: { left: number; bottom: number }, pane: { left: number; right: number; bottom: number }, width: number, gap = 12) {
    const room = pane.right - gap - chip.left;
    const shift = Math.min(0, room - width);
    // Never past the pane's left edge either
    const left = Math.max(shift, pane.left + gap - chip.left);
    const maxHeight = Math.max(160, Math.floor(pane.bottom - gap - chip.bottom - 6));
    return { left: Math.round(left), maxHeight };
}
