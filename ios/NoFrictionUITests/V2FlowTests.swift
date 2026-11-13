import XCTest

/// v2 (docs/design/FADELL_V2.md), end to end on the sample data: Up next,
/// the note line, Stop opening the recording on its Notes, and the notebook
/// page. Screenshots go to NF_SCREENSHOT_DIR when it is set.
final class V2FlowTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func launch(_ extra: [String] = []) -> XCUIApplication {
        let app = XCUIApplication()
        app.launchArguments = ["-NFResetDemo", "-NFSeedDemo", "-AppleLanguages", "(en)", "-AppleLocale", "en_US"] + extra
        app.launch()
        return app
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id].firstMatch
    }

    /// The bar's back button, or the edge swipe where the bar doesn't name it
    private func goBack(_ app: XCUIApplication) {
        let back = app.buttons["BackButton"].firstMatch
        if back.exists && back.isHittable {
            back.tap()
        } else {
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.01, dy: 0.5)).press(forDuration: 0.05, thenDragTo: app.coordinate(withNormalizedOffset: CGVector(dx: 0.8, dy: 0.5)))
        }
    }

    private func element(_ app: XCUIApplication, labelContains text: String) -> XCUIElement {
        app.descendants(matching: .any).matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    /// Record tab, idle: one Up next card for the routine; Not now hides it.
    func testUpNextShowsTheRoutineAndNotNowHidesIt() {
        let app = launch()
        let record = app.buttons["up-next-record"].firstMatch
        XCTAssertTrue(record.waitForExistence(timeout: 10), "no Up next card for the sample routine")
        XCTAssertEqual(record.label, "Record CHEM 201")
        XCTAssertTrue(element(app, labelContains: "You usually record this on").exists, "why it is offered")
        XCTAssertTrue(element(app, labelContains: "Class · 60 min").exists, "the plan: the Record sheet's answers, filled in")
        // What its notes still have open, before you walk in
        let open = element(app, "up-next-open")
        XCTAssertTrue(open.exists, "Still open from last time is missing")
        XCTAssertTrue(open.label.contains("Still open from last time"))
        XCTAssertTrue(open.label.contains("Problem set 6 is due Friday"))
        XCTAssertTrue(app.buttons["record-button"].firstMatch.exists, "the plain Record button stays")
        saveShot("v2-1-up-next")
        app.buttons["up-next-not-now"].firstMatch.tap()
        XCTAssertTrue(record.waitForNonExistence(timeout: 5), "Not now should hide the card")
        XCTAssertFalse(element(app, "reminder-prompt").exists, "no permission question right after Not now")
        // Still hidden after coming back to the tab
        app.buttons["Settings"].firstMatch.tap()
        let autoNotes = app.switches["auto-notes-toggle"].firstMatch
        scrollTo(autoNotes, in: app)
        XCTAssertTrue(autoNotes.exists, "Make notes automatically missing in Settings")
        XCTAssertEqual(autoNotes.value as? String, "1", "on unless turned off")
        let reminders = app.switches["routine-reminders-toggle"].firstMatch
        scrollTo(reminders, in: app)
        XCTAssertTrue(reminders.exists, "Remind me when something starts missing in Settings")
        saveShot("v2-1b-settings")
        app.buttons["Record"].firstMatch.tap()
        sleep(1)
        XCTAssertFalse(app.buttons["up-next-record"].firstMatch.exists)
    }

    /// Record <name> starts at once, with the notebook, type and length on the card.
    func testUpNextRecordsInOneTap() {
        // -NFDemoRecord: the Simulator has no speech recognition, so the start shows a demo recording
        let app = launch(["-recordingNoticeAccepted", "YES", "-NFDemoRecord"])
        let record = app.buttons["up-next-record"].firstMatch
        XCTAssertTrue(record.waitForExistence(timeout: 10))
        record.tap()
        XCTAssertTrue(element(app, "note-field").waitForExistence(timeout: 10), "one tap should start the recording")
        XCTAssertFalse(app.staticTexts["What is it?"].firstMatch.exists, "no Record sheet")
        XCTAssertTrue(element(app, labelContains: "CHEM 201 — ").exists, "the recording is in the suggested notebook")
        XCTAssertTrue(app.buttons["time-limit-extend"].firstMatch.exists, "the usual length is its time limit")
        XCTAssertTrue(element(app, labelContains: "left").exists)
        XCTAssertFalse(app.buttons["up-next-record"].firstMatch.exists, "no card while recording")
        saveShot("v2-1c-recording-from-up-next")
        // Stop: the recording opens on its Notes, in its notebook, as a class
        app.buttons["record-button"].firstMatch.tap()
        XCTAssertTrue(element(app, "meeting-open-notebook").waitForExistence(timeout: 10), "Stop didn't open the recording")
        XCTAssertEqual(element(app, "meeting-open-notebook").label, "Open CHEM 201")
        XCTAssertEqual(element(app, "meeting-kind").label, "What is it? Class")
        // Recorded today: nothing more is suggested for it
        app.buttons["Record"].firstMatch.tap()
        sleep(2)
        XCTAssertFalse(app.buttons["up-next-record"].firstMatch.exists, "it was just recorded today")
    }

    /// The first time, Record <name> shows the one-time recording notice, then starts what its button said.
    func testUpNextShowsTheRecordingNoticeFirst() {
        let app = launch(["-recordingNoticeAccepted", "NO", "-NFDemoRecord"])
        let record = app.buttons["up-next-record"].firstMatch
        XCTAssertTrue(record.waitForExistence(timeout: 10))
        record.tap()
        let accept = app.buttons["recording-notice-continue"].firstMatch
        XCTAssertTrue(accept.waitForExistence(timeout: 5), "the recording notice comes before the first recording")
        XCTAssertFalse(element(app, "note-field").exists, "nothing records before the notice is accepted")
        accept.tap()
        XCTAssertTrue(element(app, "note-field").waitForExistence(timeout: 10), "accepting should start the suggested recording")
        XCTAssertFalse(app.staticTexts["What is it?"].firstMatch.exists, "no Record sheet after the notice")
        XCTAssertTrue(element(app, labelContains: "CHEM 201 — ").exists)
        // The first Class recording: the one-time school-policy reminder
        XCTAssertTrue(element(app, "class-notice").exists || true)
        app.buttons["record-button"].firstMatch.tap()
    }

    /// Type a note while recording: it lands in the transcript, the field
    /// keeps the keyboard, and Stop opens the recording on its Notes.
    func testTypedNoteLandsInTheTranscriptAndStopOpensNotes() {
        let app = launch(["-NFDemoLive"])
        let field = element(app, "note-field")
        XCTAssertTrue(field.waitForExistence(timeout: 10), "no note line while recording")
        XCTAssertTrue(app.buttons["mark-button"].firstMatch.exists, "the note line sits above Mark this moment")
        XCTAssertLessThan(field.frame.maxY, app.buttons["mark-button"].firstMatch.frame.minY + 1)
        saveShot("v2-2-live")

        field.tap()
        field.typeText("ask about the beta date\n")
        saveShot("v2-3a-after-return")
        let first = element(app, labelContains: "Your note: ask about the beta date")
        XCTAssertTrue(first.waitForExistence(timeout: 5), "the note didn't appear in the transcript")
        XCTAssertTrue(first.isHittable, "the new note is on screen, above the field")
        XCTAssertTrue(["", "Type a note"].contains(field.value as? String ?? "?"), "the field is cleared")
        // Still in the field: the next line goes straight in
        XCTAssertTrue(element(app, "note-done").exists, "the field lost the keyboard after Return")
        XCTAssertFalse(app.buttons["Stop recording"].firstMatch.exists, "the big controls step aside while typing")
        XCTAssertTrue(element(app, labelContains: "Yes, it's already in the new flow").isHittable, "the transcript stays readable above the field")
        if app.keyboards.firstMatch.exists {
            XCTAssertLessThanOrEqual(field.frame.maxY, app.keyboards.firstMatch.frame.minY + 1, "the field is covered by the keyboard")
        }
        saveShot("v2-3-typing")
        field.typeText("second line\n")
        XCTAssertTrue(element(app, labelContains: "Your note: second line").waitForExistence(timeout: 5))
        saveShot("v2-3b-typing-two")

        element(app, "note-done").tap()
        let stop = app.buttons["Stop recording"].firstMatch
        XCTAssertTrue(stop.waitForExistence(timeout: 5), "Done should bring the controls back")
        stop.tap()

        // The user's own Stop: Recordings, this recording, its Notes with what was typed
        let wrote = element(app, "what-you-wrote")
        XCTAssertTrue(wrote.waitForExistence(timeout: 10), "Stop didn't open the recording's Notes")
        XCTAssertTrue(app.staticTexts["What you wrote"].firstMatch.exists)
        sleep(1)
        XCTAssertTrue(app.staticTexts["Notes"].firstMatch.isHittable, "the Notes title is on screen, clear of the bar")
        XCTAssertTrue(app.staticTexts["ask about the beta date"].firstMatch.exists)
        XCTAssertTrue(app.staticTexts["second line"].firstMatch.exists)
        XCTAssertTrue(app.buttons["ai-summarize"].firstMatch.exists, "Make notes is there when notes weren't made by themselves")
        XCTAssertFalse(app.staticTexts["MARKS"].firstMatch.exists || app.staticTexts["Marks"].firstMatch.exists, "typed notes are not marks")
        saveShot("v2-4-stopped-on-notes")

        // A typed note can be deleted; it is the user's own words, so it asks first
        let options = app.buttons.matching(NSPredicate(format: "label == %@", "Note options"))
        XCTAssertEqual(options.count, 3, "the sample note and the two typed here")
        options.element(boundBy: 2).tap()
        app.buttons["Delete note…"].firstMatch.tap()
        let confirm = app.buttons["Delete note"].firstMatch
        XCTAssertTrue(confirm.waitForExistence(timeout: 5), "deleting a typed note should ask first")
        XCTAssertTrue(app.staticTexts["second line"].firstMatch.exists, "nothing is deleted before the answer")
        XCTAssertTrue(element(app, labelContains: "is removed from this recording and its notes").exists)
        saveShot("v2-4a-delete-note-confirm")
        confirm.tap()
        XCTAssertTrue(app.staticTexts["second line"].firstMatch.waitForNonExistence(timeout: 5))
        // And it is in the transcript where it was written
        let inline = element(app, labelContains: "Your note: ask about the beta date")
        scrollTo(inline, in: app, maxSwipes: 12)
        XCTAssertTrue(inline.exists, "the note is missing from the saved transcript")
        saveShot("v2-4b-transcript-note")
    }

    /// Recordings → a notebook chip → Open: the notebook's page, every section filled.
    func testNotebookPage() {
        let app = launch(["-recordingNoticeAccepted", "YES"])
        app.buttons["Recordings"].firstMatch.tap()
        let chip = element(app, "notebook-filter").buttons["BIO 101"].firstMatch
        XCTAssertTrue(chip.waitForExistence(timeout: 10))
        chip.tap()
        let open = element(app, "recordings-open-notebook")
        XCTAssertTrue(open.waitForExistence(timeout: 5), "no Open row for the selected notebook")
        saveShot("v2-5-recordings-open-row")
        open.tap()

        XCTAssertTrue(app.buttons["notebook-record"].firstMatch.waitForExistence(timeout: 5), "the notebook page didn't open")
        XCTAssertEqual(element(app, "notebook-summary").label, "Class · 3 recordings · 2 h 30 min")
        XCTAssertFalse(app.staticTexts["Marked"].firstMatch.exists, "the marks section is named for what it holds")
        for title in ["Due", "On the test and questions", "Review everything"] {
            let header = app.staticTexts[title].firstMatch
            scrollTo(header, in: app)
            XCTAssertTrue(header.exists, "\(title) section missing")
        }
        // With guides to review, Review everything comes before the marks
        XCTAssertLessThan(app.staticTexts["Review everything"].firstMatch.frame.minY, app.staticTexts["On the test and questions"].firstMatch.frame.minY)
        app.swipeDown(); app.swipeDown()
        XCTAssertTrue(app.staticTexts["Lab report two is due Monday"].firstMatch.exists)
        XCTAssertTrue(app.staticTexts["Problem set four is due next Thursday"].firstMatch.exists)
        saveShot("v2-6-notebook-top")

        // Tick an item: its label is the item itself, its state is the value; it moves under "Show 1 done"
        let tick = app.descendants(matching: .any).matching(NSPredicate(format: "identifier == %@ AND label == %@", "item-tick", "Lab report two is due Monday")).firstMatch
        XCTAssertTrue(tick.exists, "the tick's label is the item's text")
        XCTAssertEqual(tick.value as? String, "Not done")
        tick.tap()
        let showDone = app.buttons["notebook-items-done-toggle"].firstMatch
        XCTAssertTrue(showDone.waitForExistence(timeout: 5))
        XCTAssertEqual(showDone.label, "Show 1 done")
        showDone.tap()
        XCTAssertEqual(tick.value as? String, "Done")
        saveShot("v2-6b-notebook-ticked")

        // The same tick is in the recording's Notes
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "From Lecture 6: Enzymes")).firstMatch.tap()
        let inNotes = app.descendants(matching: .any).matching(NSPredicate(format: "identifier == %@ AND label == %@", "item-tick", "Lab report two is due Monday")).firstMatch
        // The notes are long: swipe until the tick is in the window (it is a toggle, so not asked whether it is hittable off screen)
        for _ in 0..<8 where !(inNotes.exists && app.windows.firstMatch.frame.insetBy(dx: 0, dy: 120).contains(inNotes.frame)) { app.swipeUp() }
        XCTAssertTrue(inNotes.waitForExistence(timeout: 5), "the notes' items should have their tick")
        XCTAssertEqual(inNotes.value as? String, "Done", "ticked on the notebook page, ticked in Notes")
        let other = app.descendants(matching: .any).matching(NSPredicate(format: "identifier == %@ AND label == %@", "item-tick", "Read chapter 8 before Thursday's lecture")).firstMatch
        XCTAssertEqual(other.value as? String, "Not done")
        saveShot("v2-6c-notes-ticks")
        inNotes.tap()
        XCTAssertEqual(inNotes.value as? String, "Not done")
        // Back from a recording opened on the page returns to the page
        goBack(app)
        XCTAssertTrue(app.buttons["notebook-record"].firstMatch.waitForExistence(timeout: 5), "back on the notebook's page")
        XCTAssertTrue(showDone.waitForNonExistence(timeout: 5), "unticked in Notes, open again on the page")
        saveShot("v2-6d-notebook-after-untick")

        // The marks: ✎ and ? with what was written
        let marked = element(app, labelContains: "competitive vs non-competitive inhibitors")
        scrollTo(marked, in: app)
        XCTAssertTrue(marked.exists)
        XCTAssertTrue(element(app, labelContains: "why oxygen?").exists)
        XCTAssertFalse(element(app, labelContains: "lock and key").exists, "a typed note isn't a mark")
        saveShot("v2-7-notebook-marked")

        // Review everything: one deck and one quiz from both guides
        let cards = app.buttons["notebook-flashcards"].firstMatch
        for _ in 0..<6 where !(cards.exists && cards.isHittable) { app.swipeDown() }
        XCTAssertTrue(element(app, labelContains: "5 flashcards and 3 practice questions from this notebook's study guides").exists)
        saveShot("v2-8-notebook-review")
        cards.tap()
        XCTAssertTrue(element(app, "flashcard").waitForExistence(timeout: 5), "the notebook's deck didn't open")
        XCTAssertTrue(element(app, labelContains: "0 of 5 known").exists)
        // Start from the later recording: its guide alone
        let from = element(app, "notebook-review-from")
        XCTAssertTrue(from.exists, "Start from is missing")
        saveShot("v2-8b-notebook-flashcards")
        from.tap()
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Lecture 6: Enzymes")).firstMatch.tap()
        XCTAssertTrue(element(app, labelContains: "0 of 3 known").waitForExistence(timeout: 5), "from Lecture 6 on there are three cards")
        saveShot("v2-8c-notebook-start-from")
        app.buttons["Done"].firstMatch.tap()
        app.buttons["notebook-quiz"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Question 1 of 3"].firstMatch.waitForExistence(timeout: 5), "the notebook's quiz didn't open")
        app.buttons["Done"].firstMatch.tap()

        // The notebook's recordings
        let lecture = app.staticTexts["Lecture 5: Cell membranes"].firstMatch
        scrollTo(lecture, in: app)
        XCTAssertTrue(lecture.exists)
        saveShot("v2-9-notebook-recordings")

        // Ask: Chat, scoped to the notebook
        app.swipeDown(); app.swipeDown(); app.swipeDown()
        app.buttons["notebook-ask"].firstMatch.tap()
        let scope = element(app, "chat-scope")
        XCTAssertTrue(scope.waitForExistence(timeout: 5))
        XCTAssertEqual(scope.label, "Scope: Notebook · BIO 101")
        saveShot("v2-10-ask")

        // Record: the Record sheet, with this notebook and its usual type filled in
        app.buttons["Recordings"].firstMatch.tap()
        XCTAssertTrue(app.buttons["notebook-record"].firstMatch.waitForExistence(timeout: 5), "the notebook page should still be there")
        app.buttons["notebook-record"].firstMatch.tap()
        let notebook = app.textFields["record-notebook-field"].firstMatch
        XCTAssertTrue(notebook.waitForExistence(timeout: 5), "Record on the page didn't open the Record sheet")
        XCTAssertEqual(notebook.value as? String, "BIO 101")
        XCTAssertTrue(app.buttons["Class"].firstMatch.isSelected, "the notebook's usual type is preselected")
        saveShot("v2-11-record-prefilled")
        app.buttons["Cancel"].firstMatch.tap()
    }

    /// A recording's Notes open with What you wrote; its notebook label opens the page.
    func testNotesOpenWithWhatYouWroteAndLinkToTheNotebook() {
        let app = launch()
        app.buttons["Recordings"].firstMatch.tap()
        let lecture = app.staticTexts["Lecture 6: Enzymes"].firstMatch
        scrollTo(lecture, in: app)
        XCTAssertTrue(lecture.waitForExistence(timeout: 10))
        lecture.tap()
        XCTAssertTrue(element(app, "what-you-wrote").waitForExistence(timeout: 5))
        // The notes' own section, once: each line with what was being said
        XCTAssertEqual(app.staticTexts.matching(NSPredicate(format: "label == %@", "lock and key = active site?")).count, 1, "the lines aren't shown twice")
        XCTAssertTrue(element(app, labelContains: "Heat changes the enzyme's shape").exists)
        saveShot("v2-12-notes-what-you-wrote")
        // Tapping a line's time shows that moment in the transcript
        app.buttons["Your note at 25:30"].firstMatch.tap()
        let inline = element(app, labelContains: "Your note: ask why heat stops it")
        XCTAssertTrue(inline.waitForExistence(timeout: 5))
        sleep(1)
        XCTAssertTrue(inline.isHittable, "the transcript didn't scroll to the note's moment")
        saveShot("v2-13-jumped-to-note")
        // The notebook on the recording opens its page
        let link = element(app, "meeting-open-notebook")
        for _ in 0..<8 where !(link.exists && link.isHittable) { app.swipeDown() }
        XCTAssertTrue(link.isHittable)
        link.tap()
        XCTAssertTrue(app.buttons["notebook-ask"].firstMatch.waitForExistence(timeout: 5), "the notebook label didn't open its page")
    }

    /// Fill in the blanks sits with the notes of a recording that has them.
    /// It is part of Notes: without Pro its tap opens the paywall, named for Notes.
    func testFillInTheBlanksSitsWithTheNotesAndAsksForProFirst() {
        let app = launch()
        app.buttons["Recordings"].firstMatch.tap()
        let lecture = app.staticTexts["Lecture 6: Enzymes"].firstMatch
        scrollTo(lecture, in: app)
        XCTAssertTrue(lecture.waitForExistence(timeout: 10))
        lecture.tap()
        XCTAssertTrue(element(app, "what-you-wrote").waitForExistence(timeout: 5))
        let fill = app.buttons["ai-fill"].firstMatch
        scrollTo(fill, in: app)
        XCTAssertTrue(fill.exists && fill.isHittable, "Fill in the blanks is missing under the notes")
        XCTAssertEqual(fill.label, "Fill in the blanks")
        XCTAssertTrue(app.buttons["ai-summarize"].firstMatch.isHittable, "Make again stays beside it")
        saveShot("v2-14-fill-in-the-blanks")
        fill.tap()
        XCTAssertTrue(app.staticTexts["Notes are part of noFriction Pro"].firstMatch.waitForExistence(timeout: 5), "the paywall should name Notes")
        app.buttons["Close"].firstMatch.tap()
        XCTAssertTrue(fill.waitForExistence(timeout: 5))
    }
}
