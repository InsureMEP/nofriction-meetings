import SwiftData
import UserNotifications
import XCTest
@testable import noFriction

/// docs/design/FADELL_V2.md §6 "The routine": the pure function behind the
/// Up next card and its reminders, with fixed dates and time zones.
final class RoutineTests: XCTestCase {
    private func calendar(_ zone: String = "America/New_York") -> Calendar {
        var c = Calendar(identifier: .gregorian)
        c.timeZone = TimeZone(identifier: zone)!
        return c
    }

    /// A local time in `zone`
    private func at(_ month: Int, _ day: Int, _ hour: Int, _ minute: Int, zone: String = "America/New_York") -> Date {
        calendar(zone).date(from: DateComponents(year: 2026, month: month, day: day, hour: hour, minute: minute))!
    }

    private func bio(_ month: Int, _ day: Int, _ hour: Int, _ minute: Int, minutes: Int? = 50, name: String = "BIO 101",
                     kind: RecordingKind = .class) -> Routine.Recording {
        Routine.Recording(notebook: name, kind: kind, startedAt: at(month, day, hour, minute), minutes: minutes)
    }

    /// Wednesdays, 9:00 and 9:05
    private var twoWednesdays: [Routine.Recording] { [bio(10, 28, 9, 0, minutes: 60), bio(11, 4, 9, 5, minutes: 50)] }

    func testTheDatesAreTheWeekdaysTheTestsSay() {
        let c = calendar()
        XCTAssertEqual(c.component(.weekday, from: at(11, 11, 9, 0)), 4, "November 11, 2026 is a Wednesday")
        XCTAssertEqual(c.component(.weekday, from: at(10, 28, 9, 0)), 4)
        XCTAssertEqual(c.component(.weekday, from: at(11, 12, 9, 0)), 5)
    }

    func testSuggestsTheNotebookUsuallyRecordedNow() {
        let s = Routine.suggestion(twoWednesdays, now: at(11, 11, 9, 2), calendar: calendar())
        XCTAssertEqual(s, Routine.Suggestion(notebook: "BIO 101", kind: .class, limit: .minutes(60), matches: 2, usualMinute: 9 * 60))
    }

    func testWeekdayMustMatch() {
        XCTAssertNil(Routine.suggestion(twoWednesdays, now: at(11, 12, 9, 2), calendar: calendar()), "Thursday")
        // One Wednesday and one Tuesday at the same time: only one candidate
        let mixed = [bio(11, 4, 9, 0), bio(11, 3, 9, 0)]
        XCTAssertNil(Routine.suggestion(mixed, now: at(11, 11, 9, 2), calendar: calendar()))
    }

    func testWindowIsTenMinutesBeforeToTwentyFiveAfterEachStart() {
        let c = calendar()
        func suggested(_ hour: Int, _ minute: Int) -> Bool {
            Routine.suggestion(twoWednesdays, now: at(11, 11, hour, minute), calendar: c) != nil
        }
        XCTAssertFalse(suggested(8, 49), "before either window")
        XCTAssertFalse(suggested(8, 50), "only the 9:00 start matches yet")
        XCTAssertFalse(suggested(8, 54))
        XCTAssertTrue(suggested(8, 55), "10 minutes before the 9:05 start: both match")
        XCTAssertTrue(suggested(9, 0))
        XCTAssertTrue(suggested(9, 25), "25 minutes after the 9:00 start, inclusive")
        XCTAssertFalse(suggested(9, 26), "only the 9:05 start matches now")
        XCTAssertFalse(suggested(9, 31))
        // Seconds don't count: minutes of the day
        let late = c.date(byAdding: .second, value: 59, to: at(11, 11, 9, 25))!
        XCTAssertNotNil(Routine.suggestion(twoWednesdays, now: late, calendar: c))
    }

    func testNothingOnceItWasRecordedToday() {
        let today = twoWednesdays + [bio(11, 11, 8, 0, name: "bio 101")]
        XCTAssertNil(Routine.suggestion(today, now: at(11, 11, 9, 2), calendar: calendar()), "names compare ignoring case")
        // Another notebook recorded today doesn't hide this one
        let other = twoWednesdays + [bio(11, 11, 8, 0, name: "CHEM 201")]
        XCTAssertEqual(Routine.suggestion(other, now: at(11, 11, 9, 2), calendar: calendar())?.notebook, "BIO 101")
        // Today's recording is never a candidate itself
        let onlyToday = [bio(11, 4, 9, 0), bio(11, 11, 9, 0)]
        XCTAssertNil(Routine.suggestion(onlyToday, now: at(11, 11, 9, 2), calendar: calendar()))
    }

    func testTieGoesToMoreMatchesThenTheMostRecentRecording() {
        let c = calendar()
        let chem = { (m: Int, d: Int, h: Int, mi: Int) in self.bio(m, d, h, mi, name: "CHEM 201") }
        // BIO matches three times, CHEM twice
        let more = twoWednesdays + [bio(10, 21, 9, 2)] + [chem(10, 28, 9, 1), chem(11, 4, 9, 1)]
        XCTAssertEqual(Routine.suggestion(more, now: at(11, 11, 9, 2), calendar: c)?.notebook, "BIO 101")
        XCTAssertEqual(Routine.suggestion(more, now: at(11, 11, 9, 2), calendar: c)?.matches, 3)
        // Two each: CHEM was recorded most recently (yesterday, any day counts)
        let even = twoWednesdays + [chem(10, 28, 9, 1), chem(11, 4, 9, 1), chem(11, 10, 15, 0)]
        XCTAssertEqual(Routine.suggestion(even, now: at(11, 11, 9, 2), calendar: c)?.notebook, "CHEM 201")
        // Whatever order they are given in
        XCTAssertEqual(Routine.suggestion(even.reversed(), now: at(11, 11, 9, 2), calendar: c)?.notebook, "CHEM 201")
    }

    func testFadesAfterFortyTwoDays() {
        let c = calendar()
        // Wednesdays September 16 and 23: 56 and 49 days before November 11
        let old = [bio(9, 16, 9, 0), bio(9, 23, 9, 0)]
        XCTAssertNil(Routine.suggestion(old, now: at(11, 11, 9, 2), calendar: c), "both are outside the last 42 days")
        XCTAssertNotNil(Routine.suggestion(old, now: at(10, 7, 9, 2), calendar: c), "it was a routine five weeks earlier")
        // One recent recording isn't a routine; a second one is again
        XCTAssertNil(Routine.suggestion(old + [bio(11, 4, 9, 0)], now: at(11, 11, 9, 2), calendar: c))
        XCTAssertNotNil(Routine.suggestion(old + [bio(10, 28, 9, 0), bio(11, 4, 9, 0)], now: at(11, 11, 9, 2), calendar: c))
        // The edge: 42 days ago still counts (whatever its time of day), 49 doesn't
        XCTAssertNotNil(Routine.suggestion([bio(9, 30, 8, 55), bio(11, 4, 9, 0)], now: at(11, 11, 9, 2), calendar: c), "September 30 is 42 days ago")
        XCTAssertNil(Routine.suggestion([bio(9, 23, 9, 0), bio(11, 4, 9, 0)], now: at(11, 11, 9, 2), calendar: c), "September 23 is 49 days ago")
        // A recording in the future never counts
        XCTAssertNil(Routine.suggestion([bio(11, 4, 9, 0), bio(11, 18, 9, 0)], now: at(11, 11, 9, 2), calendar: c))
    }

    func testUsualTypeAndSpellingComeFromTheMostRecentRecording() {
        let recs = [bio(10, 28, 9, 0, name: "bio 101", kind: .meeting), bio(11, 4, 9, 0, name: "Bio 101", kind: .meeting),
                    bio(11, 9, 16, 0, name: "BIO 101", kind: .class)]
        let s = Routine.suggestion(recs, now: at(11, 11, 9, 2), calendar: calendar())
        XCTAssertEqual(s?.notebook, "BIO 101", "the most recent spelling")
        XCTAssertEqual(s?.kind, .class, "the type of its most recent recording, any day")
        XCTAssertEqual(s?.matches, 2)
    }

    func testUsualLengthIsTheMedianRoundedUp() {
        XCTAssertEqual(Routine.usualLength([]), .noLimit, "unknown")
        XCTAssertEqual(Routine.usualLength([0]), .noLimit, "no length")
        XCTAssertEqual(Routine.usualLength([14]), .minutes(15))
        XCTAssertEqual(Routine.usualLength([15]), .minutes(15))
        XCTAssertEqual(Routine.usualLength([16]), .minutes(30))
        XCTAssertEqual(Routine.usualLength([50, 60]), .minutes(60), "the lower of the two middle ones, like the Mac: 50")
        XCTAssertEqual(Routine.usualLength([60, 60, 20]), .minutes(60))
        XCTAssertEqual(Routine.usualLength([61]), .minutes(90))
        XCTAssertEqual(Routine.usualLength([30, 90]), .minutes(30))
        XCTAssertEqual(Routine.usualLength([91]), .noLimit, "above 90")
        XCTAssertEqual(Routine.usualLength([120, 30, 100]), .noLimit)
        XCTAssertEqual(Routine.median([9, 1, 5]), 5)
        XCTAssertEqual(Routine.median([4, 2]), 2)
        XCTAssertNil(Routine.median([]))
        // Only the matching recordings count, and one without an end is skipped
        let recs = [bio(10, 28, 9, 0, minutes: 75), bio(11, 4, 9, 0, minutes: nil), bio(11, 3, 9, 0, minutes: 10)]
        XCTAssertEqual(Routine.suggestion(recs, now: at(11, 11, 9, 2), calendar: calendar())?.limit, .minutes(90))
        // A recording that stopped at its 60-minute limit ran 60 minutes, not 61
        XCTAssertEqual(Routine.minutes(3602), 60)
        XCTAssertEqual(Routine.usualLength([Routine.minutes(3602)!, Routine.minutes(3601)!]), .minutes(60))
        XCTAssertEqual(Routine.minutes(40), 1)
        XCTAssertNil(Routine.minutes(0))
        XCTAssertNil(Routine.minutes(nil))
        // "around 9:02": the median start of the matching recordings
        let starts = [bio(10, 21, 9, 10), bio(10, 28, 9, 0), bio(11, 4, 9, 2)]
        XCTAssertEqual(Routine.suggestion(starts, now: at(11, 11, 9, 5), calendar: calendar())?.usualMinute, 9 * 60 + 2)
    }

    func testTimesAreLocalAcrossADaylightSavingChange() {
        // New York left daylight time on November 1, 2026: 9:00 on October 28
        // and 9:00 on November 4 are 23 hours apart on the clock of the world
        let recs = [bio(10, 28, 9, 0), bio(11, 4, 9, 0)]
        XCTAssertEqual(recs[1].startedAt.timeIntervalSince(recs[0].startedAt), 7 * 86_400 + 3600)
        XCTAssertNotNil(Routine.suggestion(recs, now: at(11, 11, 9, 2), calendar: calendar()), "both started at 9:00 local time")
        // Read in UTC the same recordings started at 13:00 and 14:00: no routine at 14:02
        XCTAssertNil(Routine.suggestion(recs, now: at(11, 11, 9, 2), calendar: calendar("UTC")))
        // In Tokyo it is a different day of the week for the user who records at 9:00 there
        let tokyo = [Routine.Recording(notebook: "日本語", kind: .class, startedAt: at(10, 28, 9, 0, zone: "Asia/Tokyo"), minutes: 90),
                     Routine.Recording(notebook: "日本語", kind: .class, startedAt: at(11, 4, 9, 0, zone: "Asia/Tokyo"), minutes: 90)]
        XCTAssertEqual(Routine.suggestion(tokyo, now: at(11, 11, 8, 55, zone: "Asia/Tokyo"), calendar: calendar("Asia/Tokyo"))?.limit, .minutes(90))
        XCTAssertNil(Routine.suggestion(tokyo, now: at(11, 11, 8, 55, zone: "Asia/Tokyo"), calendar: calendar()),
                     "in New York those were Tuesday evenings, and now is Tuesday too but at another time")
    }

    func testNoWrapPastMidnight() {
        let late = [bio(10, 28, 23, 50), bio(11, 4, 23, 50)]
        XCTAssertNotNil(Routine.suggestion(late, now: at(11, 11, 23, 59), calendar: calendar()))
        XCTAssertNil(Routine.suggestion(late, now: at(11, 12, 0, 5), calendar: calendar()), "after midnight it is Thursday")
    }

    // MARK: Reminders

    func testReminderFiresAtTheMedianStartOfAWeekdaysCluster() {
        let c = calendar()
        // Three starts within 35 minutes: the middle one
        let three = [bio(10, 21, 9, 20), bio(10, 28, 9, 0), bio(11, 4, 9, 4)]
        // Thursday noon: the next Wednesday is in six days
        XCTAssertEqual(Routine.reminders(three, now: at(11, 5, 12, 0), calendar: c),
                       [Routine.Reminder(notebook: "BIO 101", fireAt: at(11, 11, 9, 4))])
        // Two: the earlier one (the lower of the two middle ones, like the Mac's median)
        let recs = [bio(10, 28, 9, 0), bio(11, 4, 9, 10)]
        XCTAssertEqual(Routine.reminders(recs, now: at(11, 5, 12, 0), calendar: c).map(\.fireAt), [at(11, 11, 9, 0)])
        // Today, before it starts; nothing once the time has passed (next week is beyond 7 days)
        XCTAssertEqual(Routine.reminders(recs, now: at(11, 11, 8, 0), calendar: c).map(\.fireAt), [at(11, 11, 9, 0)])
        XCTAssertEqual(Routine.reminders(recs, now: at(11, 11, 9, 1), calendar: c), [])
    }

    func testReminderNeedsAClusterAndFadesAndSkipsARecordedDay() {
        let c = calendar()
        XCTAssertEqual(Routine.reminders([bio(10, 28, 9, 0), bio(11, 4, 9, 36)], now: at(11, 5, 12, 0), calendar: c), [],
                       "36 minutes apart is not one cluster")
        XCTAssertEqual(Routine.reminders([bio(10, 28, 9, 0), bio(11, 4, 9, 35)], now: at(11, 5, 12, 0), calendar: c).count, 1, "35 minutes apart is")
        XCTAssertEqual(Routine.reminders([bio(11, 4, 9, 0)], now: at(11, 5, 12, 0), calendar: c), [], "once is not a routine")
        XCTAssertEqual(Routine.reminders([bio(9, 16, 9, 0), bio(9, 23, 9, 0)], now: at(11, 5, 12, 0), calendar: c), [], "faded")
        // It will have faded by the day it would fire: September 30 is 42 days before November 11, October 7 keeps it alive
        XCTAssertEqual(Routine.reminders([bio(9, 30, 9, 0), bio(10, 7, 9, 0)], now: at(11, 5, 12, 0), calendar: c).count, 1)
        XCTAssertEqual(Routine.reminders([bio(9, 23, 9, 0), bio(10, 7, 9, 0)], now: at(11, 5, 12, 0), calendar: c), [])
        // Recorded today already: no reminder today
        let today = [bio(10, 28, 9, 0), bio(11, 4, 9, 0), bio(11, 11, 7, 0)]
        XCTAssertEqual(Routine.reminders(today, now: at(11, 11, 8, 0), calendar: c), [])
        // Two notebooks, two weekdays, in time order, the most recent spelling
        let two = [bio(10, 28, 9, 0), bio(11, 4, 9, 0, name: "Bio 101"),
                   bio(10, 30, 14, 0, name: "CHEM 201"), bio(11, 6, 14, 30, name: "CHEM 201")]
        XCTAssertEqual(Routine.reminders(two, now: at(11, 8, 12, 0), calendar: c),
                       [Routine.Reminder(notebook: "Bio 101", fireAt: at(11, 11, 9, 0)),
                        Routine.Reminder(notebook: "CHEM 201", fireAt: at(11, 13, 14, 0))])
        XCTAssertEqual(RoutineReminders.body("BIO 101"), "BIO 101 usually starts about now.")
    }

    func testATappedReminderShowsItsNotebookEvenOutsideTheStrictWindow() {
        let c = calendar()
        // 9:00 and 9:35, and now 8:45: before the window of either start
        let recs = [bio(10, 28, 9, 0, minutes: 50), bio(11, 4, 9, 35, minutes: 80)]
        XCTAssertEqual(Routine.reminders(recs, now: at(11, 11, 8, 0), calendar: c).map(\.fireAt), [at(11, 11, 9, 0)])
        XCTAssertNil(Routine.suggestion(recs, now: at(11, 11, 9, 0), calendar: c), "only the 9:00 start matches at 9:00")
        XCTAssertEqual(Routine.usual("bio 101", recs, now: at(11, 11, 9, 0), calendar: c),
                       Routine.Suggestion(notebook: "BIO 101", kind: .class, limit: .minutes(60), matches: 2, usualMinute: 9 * 60))
        XCTAssertNil(Routine.usual("BIO 101", recs + [bio(11, 11, 8, 0)], now: at(11, 11, 9, 0), calendar: c), "recorded today")
        XCTAssertNil(Routine.usual("CHEM 201", recs, now: at(11, 11, 9, 0), calendar: c))
    }

    func testReminderRequestCarriesTheNotebookAndAWallClockTrigger() throws {
        let c = calendar()
        let request = RoutineReminders.request(Routine.Reminder(notebook: "BIO 101", fireAt: at(11, 11, 9, 5)), index: 3, calendar: c)
        XCTAssertEqual(request.identifier, "routine-3")
        XCTAssertEqual(request.content.body, "BIO 101 usually starts about now.")
        XCTAssertEqual(request.content.categoryIdentifier, RoutineReminders.category)
        XCTAssertEqual(request.content.userInfo[RoutineReminders.notebookKey] as? String, "BIO 101")
        let trigger = try XCTUnwrap(request.trigger as? UNCalendarNotificationTrigger)
        XCTAssertFalse(trigger.repeats)
        XCTAssertEqual(trigger.dateComponents.hour, 9)
        XCTAssertEqual(trigger.dateComponents.minute, 5)
        XCTAssertEqual(trigger.dateComponents.day, 11)
    }

    // MARK: Up next

    private func event(_ id: String, _ title: String, from: Date, to: Date, allDay: Bool = false, others: Bool = false,
                       url: String? = nil) -> CalendarEventInfo {
        let me = CalendarEventInfo.Participant(email: "me@example.com", name: "Me", isOrganizer: true, isSelf: true)
        let dana = CalendarEventInfo.Participant(email: "dana@example.com", name: "Dana", isOrganizer: false, isSelf: false)
        return CalendarEventInfo(id: id, title: title, start: from, end: to, isAllDay: allDay, location: nil, notes: nil, url: url,
                                 participants: others ? [me, dana] : [me])
    }

    /// The same cases as the Mac's `current_candidates` test.
    func testCurrentEventIsTheOneThatStartedLastAndNeverAnAllDayOne() {
        let now = at(11, 11, 10, 0)
        func ev(_ id: String, _ startInMinutes: Int, _ length: Int, allDay: Bool = false) -> CalendarEventInfo {
            let start = now.addingTimeInterval(Double(startInMinutes) * 60)
            return event(id, id, from: start, to: start.addingTimeInterval(Double(length) * 60), allDay: allDay)
        }
        let events = [ev("out of office", -600, 1440, allDay: true), ev("focus block", -120, 240), ev("standup", -5, 15),
                      ev("design review", 10, 30), ev("lunch", 90, 60), ev("ended", -90, 30)]
        // On now, most recently started first (the meeting inside a longer block wins over the block); then what starts within 15 minutes
        XCTAssertEqual(CalendarMatching.currentCandidates(events, now: now).map(\.id), ["standup", "focus block", "design review"])
        XCTAssertEqual(CalendarMatching.currentCandidates(events.reversed(), now: now).map(\.id), ["standup", "focus block", "design review"])
        // Only an all-day event today: nothing is the current event, for Up next or for naming a recording
        XCTAssertEqual(CalendarMatching.currentCandidates([ev("birthday", -600, 1440, allDay: true)], now: now), [])
        XCTAssertEqual(CalendarMatching.currentCandidates([], now: now), [])
        // Soonest first among those about to start; 16 minutes away is too early
        XCTAssertEqual(CalendarMatching.currentCandidates([ev("b", 15, 30), ev("a", 3, 30), ev("c", 16, 30)], now: now).map(\.id), ["a", "b"])
    }

    func testACalendarEventIsOfferedOnlyWhenItIsAMeetingNotYetRecorded() {
        let now = at(11, 11, 9, 0)
        let from = at(11, 11, 8, 55), to = at(11, 11, 9, 50)
        func recorded(_ title: String, _ notebook: String?, _ kind: RecordingKind, _ start: Date, event: String? = nil) -> UpNext.Recorded {
            UpNext.Recorded(title: title, notebook: notebook, kind: kind, startedAt: start, calendarEventID: event)
        }
        // Alone on it, no call link, never recorded: a block in a calendar isn't a meeting
        XCTAssertNil(UpNext.event([event("e", "Deep work", from: from, to: to)], recordings: [], now: now))
        // Other people on it
        XCTAssertEqual(UpNext.event([event("e", " Acme sync ", from: from, to: to, others: true)], recordings: [], now: now)?.title, "Acme sync")
        // A call link
        XCTAssertNotNil(UpNext.event([event("e", "Call", from: from, to: to, url: "https://meet.example.com/x")], recordings: [], now: now))
        // Recorded before under that title (any case): its notebook and type come with it
        let before = [recorded("bio 101 LECTURE", "BIO 101", .class, at(11, 4, 9, 0)), recorded("BIO 101 lecture", "Old name", .meeting, at(10, 28, 9, 0))]
        let lecture = UpNext.event([event("e", "BIO 101 lecture", from: from, to: to)], recordings: before, now: now)
        XCTAssertEqual(lecture?.notebook, "BIO 101")
        XCTAssertEqual(lecture?.kind, .class)
        XCTAssertEqual(lecture?.meetingLike, true)
        // A recording of this event already exists: not offered again
        let done = [recorded("Acme sync", nil, .meeting, at(11, 11, 8, 40), event: "e")]
        XCTAssertNil(UpNext.event([event("e", "Acme sync", from: from, to: to, others: true)], recordings: done, now: now))
        // Last week's recording of the same repeating event doesn't count as this one
        let lastWeek = [recorded("Acme sync", nil, .meeting, at(11, 4, 8, 58), event: "e")]
        XCTAssertNotNil(UpNext.event([event("e", "Acme sync", from: from, to: to, others: true)], recordings: lastWeek, now: now))
        // The block that is on isn't a meeting: the meeting about to start is offered
        let both = [event("block", "Focus", from: at(11, 11, 8, 0), to: at(11, 11, 12, 0)),
                    event("m", "Design review", from: at(11, 11, 9, 10), to: at(11, 11, 9, 40), others: true)]
        XCTAssertEqual(UpNext.event(both, recordings: [], now: now)?.id, "m")
        // An all-day event, or one without a title, is never offered
        XCTAssertNil(UpNext.event([event("d", "Conference", from: at(11, 11, 0, 0), to: at(11, 12, 0, 0), allDay: true, others: true),
                                   event("u", "  ", from: from, to: to, others: true)], recordings: [], now: now))
    }

    func testCalendarComesBeforeTheRoutine() {
        let c = calendar()
        let us = Locale(identifier: "en_US")
        let now = at(11, 11, 9, 0)
        let on = UpNext.EventNow(id: "evt", title: "Acme sync", start: at(11, 11, 8, 55), end: at(11, 11, 9, 50), meetingLike: true,
                                 notebook: "Acme project", kind: nil, recorded: false)
        let routine = Routine.Suggestion(notebook: "BIO 101", kind: .class, limit: .minutes(60), matches: 2, usualMinute: 9 * 60 + 5)
        let first = UpNext.choose(event: on, routine: routine, now: now, calendar: c, locale: us, hidden: { _ in false })
        XCTAssertEqual(first?.source, .calendar)
        XCTAssertEqual(first?.name, "Acme sync")
        XCTAssertEqual(first?.detail, "On your calendar until 9:50\u{202F}AM")
        XCTAssertEqual(first?.notebook, "Acme project")
        XCTAssertEqual(first?.kind, .meeting, "a meeting unless it was recorded before as something else")
        XCTAssertEqual(first?.limit, .noLimit, "it stops when the event is over")
        XCTAssertEqual(first?.key, UpNext.calendarKey("evt", start: at(11, 11, 8, 55)))
        XCTAssertEqual(UpNext.planSummary(first!, stopsWhenOver: true), "Meeting · Stops when it's over · Acme project")
        XCTAssertEqual(UpNext.planSummary(first!, stopsWhenOver: false), "Meeting · No limit · Acme project")
        // About to start
        var soon = on
        soon.start = at(11, 11, 9, 5)
        XCTAssertEqual(UpNext.choose(event: soon, routine: nil, now: now, calendar: c, locale: us, hidden: { _ in false })?.detail,
                       "On your calendar, starts in 5 min")
        // Not a meeting, recorded, or over: the routine
        for change: (inout UpNext.EventNow) -> Void in [{ $0.meetingLike = false }, { $0.recorded = true }, { $0.end = now }] {
            var e = on
            change(&e)
            XCTAssertEqual(UpNext.choose(event: e, routine: routine, now: now, calendar: c, locale: us, hidden: { _ in false })?.source, .routine)
        }
        // The routine alone: November 11, 2026 is a Wednesday
        let learned = UpNext.choose(event: nil, routine: routine, now: now, calendar: c, locale: us, hidden: { _ in false })
        XCTAssertEqual(learned?.name, "BIO 101")
        XCTAssertEqual(learned?.detail, "You usually record this on Wednesdays around 9:05\u{202F}AM")
        XCTAssertEqual(learned?.key, "routine:bio 101")
        XCTAssertEqual(UpNext.planSummary(learned!, stopsWhenOver: true), "Class · 60 min", "the notebook is the title: not said twice")
        XCTAssertNil(UpNext.choose(event: nil, routine: nil, now: now, calendar: c, locale: us, hidden: { _ in false }))
        // Not now on the event leaves the routine; on both, nothing
        XCTAssertEqual(UpNext.choose(event: on, routine: routine, now: now, calendar: c, locale: us, hidden: { $0.hasPrefix("calendar:") })?.source, .routine)
        XCTAssertNil(UpNext.choose(event: on, routine: routine, now: now, calendar: c, locale: us, hidden: { _ in true }))
    }

    func testNotNowHidesUntilTheNextLocalDayAndStoresNoNames() throws {
        let suite = "nf-upnext-\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }
        let c = calendar()
        let key = UpNext.routineKey("BIO 101")
        XCTAssertFalse(UpNextDismissals.isHidden(key, now: at(11, 11, 9, 0), calendar: c, defaults: defaults))
        let eventKey = UpNext.calendarKey("evt", start: at(11, 11, 9, 0))
        UpNextDismissals.hide([key, eventKey], now: at(11, 11, 9, 0), calendar: c, defaults: defaults)
        XCTAssertTrue(UpNextDismissals.isHidden(key, now: at(11, 11, 9, 1), calendar: c, defaults: defaults))
        XCTAssertTrue(UpNextDismissals.isHidden(UpNext.routineKey("bio 101"), now: at(11, 11, 23, 59), calendar: c, defaults: defaults))
        XCTAssertTrue(UpNextDismissals.isHidden(eventKey, now: at(11, 11, 12, 0), calendar: c, defaults: defaults))
        XCTAssertFalse(UpNextDismissals.isHidden(UpNext.calendarKey("evt", start: at(11, 18, 9, 0)), now: at(11, 11, 12, 0), calendar: c, defaults: defaults),
                       "next week's occurrence is another suggestion")
        XCTAssertFalse(UpNextDismissals.isHidden(UpNext.routineKey("CHEM 201"), now: at(11, 11, 9, 1), calendar: c, defaults: defaults))
        XCTAssertFalse(UpNextDismissals.isHidden(key, now: at(11, 12, 0, 0), calendar: c, defaults: defaults), "the next local day")
        XCTAssertTrue(UpNextDismissals.anyToday(now: at(11, 11, 18, 0), calendar: c, defaults: defaults))
        XCTAssertFalse(UpNextDismissals.anyToday(now: at(11, 12, 0, 0), calendar: c, defaults: defaults), "the next local day")
        // 23:30 in New York is already tomorrow in Tokyo
        XCTAssertFalse(UpNextDismissals.isHidden(key, now: at(11, 11, 23, 30), calendar: calendar("Asia/Tokyo"), defaults: defaults))
        // Hashes and the day only
        let stored = try XCTUnwrap(defaults.dictionary(forKey: UpNextDismissals.key) as? [String: String])
        XCTAssertEqual(stored.count, 2)
        XCTAssertTrue(stored.keys.allSatisfy { $0.count == 64 && !$0.lowercased().contains("bio") })
        XCTAssertEqual(Set(stored.values), ["2026-11-11"])
        // Yesterday's entries go when something is hidden again
        UpNextDismissals.hide([UpNext.routineKey("CHEM 201")], now: at(11, 12, 8, 0), calendar: c, defaults: defaults)
        XCTAssertEqual((defaults.dictionary(forKey: UpNextDismissals.key) as? [String: String])?.count, 1)
    }

    // MARK: Placement in a transcript

    func testAMomentFollowsTheLineBeingSpoken() {
        let t = Date(timeIntervalSince1970: 1_790_000_000)
        let rows = [t, t.addingTimeInterval(10), t.addingTimeInterval(20)]
        XCTAssertEqual(TranscriptPlacement.host(of: t.addingTimeInterval(-5), rows: rows), 0, "before any line: after the first")
        XCTAssertEqual(TranscriptPlacement.host(of: t.addingTimeInterval(10), rows: rows), 1)
        XCTAssertEqual(TranscriptPlacement.host(of: t.addingTimeInterval(15), rows: rows), 1)
        XCTAssertEqual(TranscriptPlacement.host(of: t.addingTimeInterval(99), rows: rows), 2)
        XCTAssertNil(TranscriptPlacement.host(of: t, rows: []))
        let hosted = TranscriptPlacement.hosted([t.addingTimeInterval(12), t.addingTimeInterval(14), t.addingTimeInterval(30)], rows: rows)
        XCTAssertEqual(hosted, [1: [0, 1], 2: [2]])
    }
}

/// Planning the reminders from the saved recordings. Permission is never
/// asked for here; what is scheduled depends on what iOS already allows.
@MainActor
final class RoutineRemindersTests: XCTestCase {
    private var container: ModelContainer!

    override func setUp() async throws {
        container = try ModelContainer(for: Schema(Storage.modelTypes), configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        for key in [RoutineReminders.settingKey, RoutineReminders.promptAnsweredKey, UpNextDismissals.key] {
            UserDefaults.standard.removeObject(forKey: key)
        }
    }

    override func tearDown() async throws {
        UserDefaults.standard.set(false, forKey: RoutineReminders.settingKey)
        await RoutineReminders.replan(context: container.mainContext)   // cancels anything this test scheduled
        for key in [RoutineReminders.settingKey, RoutineReminders.promptAnsweredKey, UpNextDismissals.key] {
            UserDefaults.standard.removeObject(forKey: key)
        }
        container = nil
    }

    private func pending() async -> [UNNotificationRequest] {
        await UNUserNotificationCenter.current().pendingNotificationRequests().filter { $0.identifier.hasPrefix(RoutineReminders.idPrefix) }
    }

    func testPlanFollowsTheRoutineTheSettingAndWhatIOSAllows() async throws {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "America/New_York")!
        func at(_ month: Int, _ day: Int, _ hour: Int, _ minute: Int) -> Date {
            calendar.date(from: DateComponents(year: 2026, month: month, day: day, hour: hour, minute: minute))!
        }
        let context = container.mainContext
        let now = at(11, 11, 8, 0)
        // No recordings: no routine, nothing to ask
        var plan = await RoutineReminders.replan(context: context, now: now, calendar: calendar)
        XCTAssertEqual(plan, RoutineReminders.Plan())
        // CHEM 201 on two earlier Wednesdays at 9:00
        for start in [at(10, 28, 9, 0), at(11, 4, 9, 2)] {
            let m = Meeting(title: "CHEM 201", startedAt: start)
            m.endedAt = start.addingTimeInterval(3000)
            m.courseName = "CHEM 201"
            m.kind = .class
            context.insert(m)
        }
        try context.save()
        XCTAssertEqual(Routine.recordings(in: context, now: now).map(\.minutes), [50, 50])
        plan = await RoutineReminders.replan(context: context, now: now, calendar: calendar)
        XCTAssertEqual(plan.next, "CHEM 201", "a routine exists")
        let status = await UNUserNotificationCenter.current().notificationSettings().authorizationStatus
        let scheduled = await pending()
        switch status {
        case .notDetermined:
            XCTAssertTrue(plan.shouldAsk, "ask in context, on the Record tab: never by itself")
            XCTAssertTrue(scheduled.isEmpty, "nothing is scheduled before iOS allows it")
        case .denied:
            XCTAssertFalse(plan.shouldAsk, "denied: do nothing")
            XCTAssertTrue(scheduled.isEmpty)
        default:
            XCTAssertFalse(plan.shouldAsk)
            XCTAssertEqual(scheduled.map(\.content.body), ["CHEM 201 usually starts about now."])
        }
        // Answered once (Continue or Not now): not asked again
        RoutineReminders.declinePrompt()
        plan = await RoutineReminders.replan(context: context, now: now, calendar: calendar)
        XCTAssertFalse(plan.shouldAsk)
        UserDefaults.standard.removeObject(forKey: RoutineReminders.promptAnsweredKey)
        // Not now on its card today: no reminder today, and no permission question right after
        UpNextDismissals.hide([UpNext.routineKey("CHEM 201")], now: now, calendar: calendar)
        plan = await RoutineReminders.replan(context: context, now: now, calendar: calendar)
        XCTAssertEqual(plan, RoutineReminders.Plan(), "today was its only day this week")
        // Another notebook's routine comes next (Thursdays): still no question on the day of a Not now
        for start in [at(10, 29, 10, 0), at(11, 5, 10, 1)] {
            let m = Meeting(title: "BIO 101", startedAt: start)
            m.endedAt = start.addingTimeInterval(3000)
            m.courseName = "BIO 101"
            m.kind = .class
            context.insert(m)
        }
        try context.save()
        plan = await RoutineReminders.replan(context: context, now: now, calendar: calendar)
        XCTAssertEqual(plan.next, "BIO 101")
        XCTAssertFalse(plan.shouldAsk, "not on the day of a Not now, whichever notebook is next")
        // The setting off: nothing planned, nothing pending
        UserDefaults.standard.removeObject(forKey: UpNextDismissals.key)
        UserDefaults.standard.set(false, forKey: RoutineReminders.settingKey)
        plan = await RoutineReminders.replan(context: context, now: now, calendar: calendar)
        XCTAssertEqual(plan, RoutineReminders.Plan())
        let left = await pending()
        XCTAssertTrue(left.isEmpty)
    }
}
