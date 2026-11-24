import XCTest

/// Settings → Storage on the sample data, with one recording given audio and
/// a camera-sized photo (-NFSpaceDemo): the size in the Recordings list, the
/// Storage screen, Make photos smaller and Remove audio. Screenshots go to
/// NF_SCREENSHOT_DIR when it is set.
final class StorageFlowTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    private func element(_ app: XCUIApplication, _ id: String) -> XCUIElement {
        app.descendants(matching: .any)[id].firstMatch
    }

    func testStorageShowsWhatTakesSpaceAndFreesIt() {
        let app = XCUIApplication()
        // The two Storage settings at their defaults, whatever an earlier run left
        app.launchArguments = ["-NFResetDemo", "-NFSeedDemo", "-NFSpaceDemo", "-AppleLanguages", "(en)", "-AppleLocale", "en_US",
                               "-space.photoSize", "smaller", "-space.keepAudioDays", "0"]
        app.launch()

        // Recordings: the size of a recording that takes room, and the total under the list
        app.buttons["Recordings"].firstMatch.tap()
        let rowSize = element(app, "meeting-row-size")
        XCTAssertTrue(rowSize.waitForExistence(timeout: 15), "no size on the recording with audio and a photo")
        XCTAssertTrue(rowSize.label.contains("megabytes"), "the size is read as words: \(rowSize.label)")
        saveShot("storage-1-recordings")
        let summary = element(app, "recordings-storage")
        scrollTo(summary, in: app)
        XCTAssertTrue(summary.waitForExistence(timeout: 5), "no total under the Recordings list")
        XCTAssertTrue(summary.label.contains("recordings, "), summary.label)
        XCTAssertTrue(summary.label.contains(" on this "), summary.label)
        saveShot("storage-2-recordings-total")
        summary.tap()

        // Storage: the total, the breakdown and both settings
        let total = element(app, "space-total")
        XCTAssertTrue(total.waitForExistence(timeout: 5), "the Storage screen didn't open")
        XCTAssertTrue(total.label.hasPrefix("noFriction uses "), total.label)
        XCTAssertTrue(total.label.contains("free on this"), total.label)
        for row in ["Audio", "Photos and screens", "Everything else"] {
            XCTAssertTrue(app.staticTexts[row].firstMatch.exists, "\(row) missing")
        }
        let shrink = element(app, "space-shrink")
        XCTAssertTrue(shrink.waitForExistence(timeout: 10), "Make photos smaller isn't offered for a camera-sized photo")
        XCTAssertTrue(shrink.label.contains("saves about"), shrink.label)
        saveShot("storage-3-screen")

        // Make photos smaller: the bytes freed, then nothing more to offer
        shrink.tap()
        let result = element(app, "space-shrink-result")
        XCTAssertTrue(result.waitForExistence(timeout: 30), "no result after Make photos smaller")
        XCTAssertTrue(result.label.hasPrefix("Freed "), result.label)
        XCTAssertTrue(shrink.waitForNonExistence(timeout: 10), "still offered with nothing left to save")
        saveShot("storage-4-freed")

        let keepAudio = element(app, "space-keep-audio")
        scrollTo(keepAudio, in: app)
        XCTAssertTrue(keepAudio.exists, "Keep audio for missing")
        XCTAssertTrue(element(app, "space-photo-size").exists, "New photos missing")

        // Largest recordings: Remove audio asks first, then keeps the recording
        let row = element(app, "space-row")
        scrollTo(row, in: app)
        XCTAssertTrue(row.exists, "no Largest recordings")
        XCTAssertTrue(row.label.contains("Brightwater pilot kickoff"), row.label)
        saveShot("storage-5-largest")
        let menu = element(app, "space-row-menu")
        menu.tap()
        XCTAssertTrue(app.buttons["Make photos smaller"].firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["Delete recording"].firstMatch.exists)
        saveShot("storage-6-menu")
        app.buttons["Remove audio"].firstMatch.tap()
        XCTAssertTrue(app.staticTexts["The transcript, notes and photos are kept. The audio can't be played again."].firstMatch
            .waitForExistence(timeout: 5), "no confirmation before removing audio")
        saveShot("storage-7-confirm")
        app.buttons["Remove audio"].firstMatch.tap()
        // Its audio row is gone from the list's detail; the recording is still there, with its photo
        let audioGone = NSPredicate(format: "NOT (label CONTAINS[c] %@)", "audio")
        expectation(for: audioGone, evaluatedWith: row)
        waitForExpectations(timeout: 10)
        XCTAssertTrue(row.label.contains("Brightwater pilot kickoff"), "the recording is kept: \(row.label)")
        saveShot("storage-8-audio-removed")

        // The recording: no playback, one line saying why, everything else there
        app.navigationBars["Storage"].buttons.firstMatch.tap()   // back to the list
        let recording = app.staticTexts["Brightwater pilot kickoff"].firstMatch
        for _ in 0..<6 where !(recording.exists && recording.isHittable) { app.swipeDown() }
        XCTAssertTrue(recording.waitForExistence(timeout: 5))
        recording.tap()
        let removed = element(app, "audio-removed")
        XCTAssertTrue(removed.waitForExistence(timeout: 5), "no line saying the audio was removed")
        XCTAssertTrue(removed.label.hasPrefix("Audio removed to save space on "), removed.label)
        XCTAssertTrue(removed.label.hasSuffix("The transcript is kept."), removed.label)
        XCTAssertFalse(element(app, "playback-button").exists)
        saveShot("storage-9-recording")

        // Settings: the Storage row with the total
        app.buttons["Settings"].firstMatch.tap()
        let settingsRow = element(app, "space-settings-row")
        scrollTo(settingsRow, in: app)
        XCTAssertTrue(settingsRow.exists, "Storage missing in Settings")
        saveShot("storage-10-settings")
        settingsRow.tap()
        XCTAssertTrue(element(app, "space-total").waitForExistence(timeout: 5))
    }
}
