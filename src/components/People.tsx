// noFriction Meetings - People
//
// Who was in the meeting, from the calendar invite — and their LinkedIn.
// LinkedIn has no lookup API, so linking is one honest step: "Find" opens a
// prefilled LinkedIn search; paste the profile URL back and it sticks to
// that person for every meeting they're in.
//
// On a recording it is one chip in the title row ("Maya, Jordan +8"); the
// list opens over the page from it. It never takes room from the recording:
// a ten-person invite once filled the page and left Rewind a strip (owner,
// 2026-11-18: "The point is rewind, so show me the content").

import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
    getCalendarAccessStatus,
    getMeetingPeople,
    linkedinSearchUrl,
    listPeople,
    setPersonLinkedin,
    syncCalendar,
    type CalendarAccess,
    type MeetingPeople as MeetingPeopleData,
    type Person,
} from "../lib/tauri";
import { chipLabel, chipTitle, eventHeading, popoverPlace, scheduledLine } from "../lib/peopleChip";
import { CalendarIcon, ChevronDownIcon, PeopleIcon, SearchIcon } from "./icons";
import "./People.css";

const open_ = (url: string) => openUrl(url).catch((e) => console.error("open failed", e));

function initials(p: Person) {
    const src = p.name || p.email;
    const parts = src.split(/[\s@._-]+/).filter(Boolean);
    return ((parts[0]?.[0] ?? "") + (parts[1]?.[0] ?? "")).toUpperCase() || "?";
}

function useRefreshOnPeopleUpdated(refresh: () => void) {
    useEffect(() => {
        let off: (() => void) | null = null;
        let disposed = false;
        listen("people_updated", () => refresh()).then((fn) => (disposed ? fn() : (off = fn)));
        return () => {
            disposed = true;
            off?.();
        };
    }, [refresh]);
}

// ── LinkedIn control ────────────────────────────────────────────────────

function LinkedInControl({ person, onSaved }: { person: Person; onSaved: (url: string | null) => void }) {
    const [editing, setEditing] = useState(false);
    const [value, setValue] = useState("");
    const [error, setError] = useState<string | null>(null);

    const save = async (url: string | null) => {
        try {
            const saved = await setPersonLinkedin(person.id, url);
            onSaved(saved);
            setEditing(false);
            setError(null);
        } catch (e) {
            setError(String(e));
        }
    };

    if (editing) {
        return (
            <form
                className="li-edit"
                onSubmit={(e) => {
                    e.preventDefault();
                    save(value);
                }}
            >
                <input
                    autoFocus
                    value={value}
                    placeholder="linkedin.com/in/…"
                    onChange={(e) => setValue(e.target.value)}
                    onKeyDown={(e) => e.key === "Escape" && setEditing(false)}
                    aria-label={`LinkedIn profile URL for ${person.name ?? person.email}`}
                />
                <button type="submit">Save</button>
                {error && <span className="li-edit__error">{error}</span>}
            </form>
        );
    }

    if (person.linkedin_url) {
        return (
            <div className="li">
                <button className="li__open" onClick={() => open_(person.linkedin_url!)} type="button" title={person.linkedin_url}>
                    <span className="li__badge" aria-hidden>in</span>
                    Profile
                </button>
                <button
                    className="li__ghost"
                    onClick={() => {
                        setValue(person.linkedin_url ?? "");
                        setEditing(true);
                    }}
                    type="button"
                    title="Change LinkedIn link"
                >
                    Edit
                </button>
            </div>
        );
    }

    return (
        <div className="li">
            <button className="li__ghost" onClick={() => open_(linkedinSearchUrl(person))} type="button" title="Search LinkedIn for this person">
                <SearchIcon size={12} /> Find
            </button>
            <button
                className="li__ghost"
                onClick={() => {
                    setValue("");
                    setEditing(true);
                }}
                type="button"
            >
                Paste link
            </button>
        </div>
    );
}

function PersonRow({ person, onChange, showStats }: { person: Person; onChange: (p: Person) => void; showStats?: boolean }) {
    return (
        <li className="person">
            <span className="person__avatar" aria-hidden>{initials(person)}</span>
            <div className="person__who">
                <span className="person__name">
                    {person.name ?? person.email}
                    {person.role === "organizer" && <span className="person__tag">Organizer</span>}
                </span>
                <span className="person__sub">
                    {[person.company, person.email].filter(Boolean).join(" · ")}
                    {showStats && person.meeting_count > 0 && (
                        <>
                            {" · "}
                            {person.meeting_count} meeting{person.meeting_count === 1 ? "" : "s"}
                            {person.last_met && `, last ${new Date(person.last_met).toLocaleDateString([], { month: "short", day: "numeric" })}`}
                        </>
                    )}
                </span>
            </div>
            <LinkedInControl person={person} onSaved={(url) => onChange({ ...person, linkedin_url: url })} />
        </li>
    );
}

// ── Connect-calendar prompt ─────────────────────────────────────────────

function ConnectCalendar({ access, onSynced }: { access: CalendarAccess | null; onSynced: () => void }) {
    const [busy, setBusy] = useState(false);
    const [msg, setMsg] = useState<string | null>(null);

    const connect = async () => {
        setBusy(true);
        try {
            const r = await syncCalendar();
            if (r.access !== "authorized") {
                setMsg("Calendar access is off. Turn on noFriction in System Settings → Privacy & Security → Calendars, then try again.");
                invoke("open_system_settings", { pane: "calendar" }).catch((e) => console.error("open failed", e));
            } else {
                setMsg(r.report ? `Linked ${r.report.meetings_linked} of ${r.report.meetings_checked} recordings to your calendar.` : null);
                onSynced();
            }
        } catch (e) {
            setMsg(String(e));
        } finally {
            setBusy(false);
        }
    };

    return (
        <div className="cal-connect">
            <CalendarIcon size={18} />
            <div className="cal-connect__text">
                <strong>{access === "denied" ? "Calendar access is off" : "Connect your calendar"}</strong>
                <span>{msg ?? "Recordings get their real titles, times and attendees — then link each person's LinkedIn."}</span>
            </div>
            <button onClick={connect} disabled={busy} type="button">
                {busy ? "Connecting…" : access === "denied" ? "Open Settings" : "Connect"}
            </button>
        </div>
    );
}

// ── On a recording: one chip, the list behind it ────────────────────────

/** How wide the list opens (People.css `.mpeople__pop`) */
const POP_WIDTH = 440;

export function MeetingPeople({ meetingId, recordingTitle = null }: { meetingId: string; recordingTitle?: string | null }) {
    const [data, setData] = useState<MeetingPeopleData | null>(null);
    const [access, setAccess] = useState<CalendarAccess | null>(null);
    const [open, setOpen] = useState(false);
    const [place, setPlace] = useState<{ left: number; maxHeight: number } | null>(null);
    const wrap = useRef<HTMLSpanElement>(null);
    const chip = useRef<HTMLButtonElement>(null);
    const popId = useId();

    const load = useCallback(() => {
        getMeetingPeople(meetingId).then(setData).catch(() => setData({ details: null, people: [] }));
        getCalendarAccessStatus().then(setAccess).catch(() => {});
    }, [meetingId]);

    useEffect(() => {
        // Another recording: nothing of the last one stays up
        setData(null);
        setOpen(false);
        load();
    }, [load]);
    useRefreshOnPeopleUpdated(load);

    // Open inside the recording's pane, and as tall as fits under the chip
    useLayoutEffect(() => {
        if (!open) return;
        const measure = () => {
            const c = chip.current?.getBoundingClientRect();
            const pane = (wrap.current?.closest(".recording-detail") ?? document.body).getBoundingClientRect();
            if (c) setPlace(popoverPlace(c, pane, POP_WIDTH));
        };
        measure();
        window.addEventListener("resize", measure);
        return () => window.removeEventListener("resize", measure);
    }, [open]);

    // A click anywhere else, or Esc, closes it
    useEffect(() => {
        if (!open) return;
        const outside = (e: PointerEvent) => {
            if (!wrap.current?.contains(e.target as Node)) setOpen(false);
        };
        const key = (e: KeyboardEvent) => {
            // Esc in the LinkedIn field cancels that edit, not the list
            if (e.key !== "Escape" || (e.target as HTMLElement | null)?.tagName === "INPUT") return;
            setOpen(false);
            chip.current?.focus();
        };
        document.addEventListener("pointerdown", outside);
        document.addEventListener("keydown", key);
        return () => {
            document.removeEventListener("pointerdown", outside);
            document.removeEventListener("keydown", key);
        };
    }, [open]);

    if (!data) return null;
    const people = data.people.filter((p) => !p.is_self);
    const d = data.details;
    // Nothing on the calendar at that time: nothing to say about it
    if (!d && (access === null || access === "authorized")) return null;

    const when = d?.scheduled_start
        ? `${new Date(d.scheduled_start).toLocaleString([], { weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" })}${
              d.scheduled_end ? ` – ${new Date(d.scheduled_end).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : ""
          }`
        : null;
    const label = !d ? (access === "denied" ? "Calendar access is off" : "Connect your calendar") : chipLabel(people) || "Calendar event";
    const title = !d ? "Recordings get their titles, times and attendees from your calendar" : chipTitle(people, d.event_title);

    return (
        <span className="mpeople" ref={wrap}>
            <button
                ref={chip}
                type="button"
                className={`rec-row rec-row--btn mpeople__chip ${d ? "" : "is-empty"} ${open ? "is-open" : ""}`}
                aria-haspopup="dialog"
                aria-expanded={open}
                aria-controls={open ? popId : undefined}
                aria-label={title}
                title={title}
                onClick={() => setOpen((o) => !o)}
            >
                {d && people.length > 0 ? <PeopleIcon size={14} /> : <CalendarIcon size={14} />}
                <span className="mpeople__label">{label}</span>
                <ChevronDownIcon size={12} className="mpeople__chev" />
            </button>
            {open && (
                <div
                    id={popId}
                    className="mpeople__pop"
                    role="dialog"
                    aria-label={d ? "The calendar event and who was in it" : "Connect your calendar"}
                    style={place ? { left: place.left, maxHeight: place.maxHeight } : { visibility: "hidden" }}
                >
                    {!d ? (
                        <ConnectCalendar access={access} onSynced={load} />
                    ) : (
                        <>
                            <header className="mpeople__head">
                                <div className="mpeople__event">
                                    {/* The event's name only when it isn't the recording's: it is right above */}
                                    {eventHeading(d.event_title, recordingTitle) && <span className="mpeople__title">{d.event_title}</span>}
                                    <span className="mpeople__meta">{scheduledLine(when, d.location, d.calendar_name)}</span>
                                </div>
                                {d.meeting_url && (
                                    <button className="li__ghost" onClick={() => open_(d.meeting_url!)} type="button">Join link</button>
                                )}
                            </header>
                            {people.length > 0 ? (
                                <ul className="people-list">
                                    {people.map((p) => (
                                        <PersonRow
                                            key={p.id}
                                            person={p}
                                            onChange={(np) => setData((cur) => cur && { ...cur, people: cur.people.map((x) => (x.id === np.id ? np : x)) })}
                                        />
                                    ))}
                                </ul>
                            ) : (
                                <p className="mpeople__none">No other attendees on the invite.</p>
                            )}
                            {d.notes && (
                                <details className="mpeople__notes">
                                    <summary>Invite notes</summary>
                                    <p>{d.notes}</p>
                                </details>
                            )}
                        </>
                    )}
                </div>
            )}
        </span>
    );
}

// ── Directory ───────────────────────────────────────────────────────────

export function PeopleDirectory() {
    const [people, setPeople] = useState<Person[] | null>(null);
    const [access, setAccess] = useState<CalendarAccess | null>(null);
    const [query, setQuery] = useState("");
    const [onlyUnlinked, setOnlyUnlinked] = useState(false);
    const [syncing, setSyncing] = useState(false);

    const load = useCallback(() => {
        listPeople().then(setPeople).catch(() => setPeople([]));
        getCalendarAccessStatus().then(setAccess).catch(() => {});
    }, []);
    useEffect(load, [load]);
    useRefreshOnPeopleUpdated(load);

    const shown = useMemo(() => {
        const q = query.trim().toLowerCase();
        return (people ?? []).filter(
            (p) =>
                (!onlyUnlinked || !p.linkedin_url) &&
                (!q || [p.name, p.email, p.company].some((f) => f?.toLowerCase().includes(q)))
        );
    }, [people, query, onlyUnlinked]);

    const resync = async () => {
        setSyncing(true);
        try {
            await syncCalendar(true);
            load();
        } finally {
            setSyncing(false);
        }
    };

    return (
        <div className="pdir">
            {access !== "authorized" && <ConnectCalendar access={access} onSynced={load} />}
            <div className="pdir__bar">
                <input
                    className="pdir__search"
                    placeholder="Search people or companies"
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                />
                <label className="pdir__toggle">
                    <input type="checkbox" checked={onlyUnlinked} onChange={(e) => setOnlyUnlinked(e.target.checked)} />
                    No LinkedIn yet
                </label>
                {access === "authorized" && (
                    <button className="li__ghost" onClick={resync} disabled={syncing} type="button">
                        {syncing ? "Syncing…" : "Re-sync calendar"}
                    </button>
                )}
            </div>
            {people === null ? null : shown.length === 0 ? (
                <p className="pdir__empty">
                    {people.length === 0 ? "People from your meeting invites will appear here." : "No one matches."}
                </p>
            ) : (
                <ul className="people-list">
                    {shown.map((p) => (
                        <PersonRow
                            key={p.id}
                            person={p}
                            showStats
                            onChange={(np) => setPeople((cur) => cur && cur.map((x) => (x.id === np.id ? np : x)))}
                        />
                    ))}
                </ul>
            )}
        </div>
    );
}
