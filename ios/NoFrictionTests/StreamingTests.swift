import SwiftData
import XCTest
@testable import noFriction

// Streamed answers (docs/TOPICS_AND_CHAT.md "Streamed answers"): the stream
// parser, what is shown while an answer is written, the client against a
// stub (no request leaves the process), and the Chat tab's holder with the
// purge rule.

/// What a sink was given, in order.
private final class Seen: @unchecked Sendable {
    private let lock = NSLock()
    private var all: [String] = []
    var texts: [String] { lock.withLock { all } }
    func add(_ s: String) { lock.withLock { all.append(s) } }
}

/// Whether each request a stub received asked for a stream, in order.
private final class Asked: @unchecked Sendable {
    private let lock = NSLock()
    private var all: [Bool?] = []
    var streams: [Bool?] { lock.withLock { all } }
    func add(_ stream: Bool?) { lock.withLock { all.append(stream) } }
}

final class StreamParsingTests: XCTestCase {
    private func feed(_ lines: [String]) -> ResponseParser.StreamedText {
        var p = ResponseParser.StreamedText()
        for l in lines { _ = p.line(l) }
        return p
    }

    func testChunksBecomeTheAnswerAndReasoningIsSkipped() throws {
        var p = ResponseParser.StreamedText()
        XCTAssertFalse(p.line(": keep-alive"))
        XCTAssertFalse(p.line("event: message"))
        XCTAssertFalse(p.line(#"data: {"choices":[{"delta":{"role":"assistant","content":""}}]}"#))
        XCTAssertFalse(p.line(#"data: {"choices":[{"delta":{"reasoning_content":"hmm"}}]}"#))
        XCTAssertTrue(p.line(#"data: {"choices":[{"delta":{"content":"Ship "}}]}"#))
        XCTAssertTrue(p.line(#"data:{"choices":[{"delta":{"content":[{"type":"text","text":"Friday ✓"}]}}]}"#))
        XCTAssertFalse(p.line(#"data: {"choices":[{"delta":{},"finish_reason":"stop"}]}"#))
        XCTAssertFalse(p.line(#"data: {"choices":[],"usage":{"total_tokens":9}}"#))
        XCTAssertFalse(p.line("data: not json"))
        XCTAssertFalse(p.line("data: [DONE]"))
        XCTAssertTrue(p.done)
        XCTAssertEqual(p.finish, "stop")
        XCTAssertEqual(try p.result(secrets: []), "Ship Friday ✓")
    }

    func testAStreamThatBreaksFailsOrSaysNothingIsAnError() {
        // A body that just stops is a broken connection, never half an answer
        let cut = feed([#"data: {"choices":[{"delta":{"content":"They decided to"}}]}"#])
        XCTAssertThrowsError(try cut.result(secrets: [])) {
            guard case .unreachable = $0 as? AIError else { return XCTFail("\($0)") }
        }
        // The end marker alone is enough
        XCTAssertEqual(try feed([#"data: {"choices":[{"delta":{"content":"Whole."}}]}"#, "data: [DONE]"]).result(secrets: []), "Whole.")
        // An error inside the stream keeps its kind and loses the key
        let rate = feed([#"data: {"choices":[{"delta":{"content":"Half"}}]}"#,
                         #"data: {"error":{"type":"rate_limit_error","message":"Slow down, key stub-key-value-1234"}}"#])
        XCTAssertThrowsError(try rate.result(secrets: ["stub-key-value-1234"])) {
            guard case .noCredit(let m) = $0 as? AIError else { return XCTFail("\($0)") }
            XCTAssertTrue(m.contains("Slow down"))
            XCTAssertFalse(m.contains("stub-key-value-1234"))
        }
        XCTAssertThrowsError(try feed([#"data: {"error":{"code":"invalid_api_key","message":"Bad key"}}"#]).result(secrets: [])) {
            XCTAssertEqual($0 as? AIError, .wrongKey("Bad key"))
        }
        XCTAssertThrowsError(try feed([#"data: {"error":{"message":"Overloaded"}}"#]).result(secrets: [])) {
            XCTAssertEqual($0 as? AIError, .server(500, "Overloaded"))
        }
        XCTAssertThrowsError(try feed([#"data: {"choices":[{"delta":{},"finish_reason":"length"}]}"#]).result(secrets: [])) {
            XCTAssertEqual($0 as? AIError, .truncated)
        }
        XCTAssertThrowsError(try feed(["data: [DONE]"]).result(secrets: [])) {
            XCTAssertEqual($0 as? AIError, .emptyAnswer)
        }
        // Only thinking: nothing to show, nothing to return
        XCTAssertThrowsError(try feed([#"data: {"choices":[{"delta":{"content":"<think>plan</think>"},"finish_reason":"length"}]}"#]).result(secrets: [])) {
            XCTAssertEqual($0 as? AIError, .truncated)
        }
    }

    func testThinkingIsNeverShownWhileItIsWritten() {
        XCTAssertEqual(ResponseParser.visibleSoFar("Plain so far "), "Plain so far")
        XCTAssertEqual(ResponseParser.visibleSoFar("<think>plan the ans"), "")
        XCTAssertEqual(ResponseParser.visibleSoFar("<think>plan</think>Ship"), "Ship")
        XCTAssertEqual(ResponseParser.visibleSoFar("Ship <think>hmm</think>Friday <think>more"), "Ship Friday")
        for partial in ["Ship <", "Ship <t", "Ship <thin", "Ship <think"] {
            XCTAssertEqual(ResponseParser.visibleSoFar(partial), "Ship", partial)
        }
        XCTAssertEqual(ResponseParser.visibleSoFar("a < b and <b"), "a < b and <b")
        XCTAssertEqual(ResponseParser.visibleSoFar("x <thing"), "x <thing")
        XCTAssertTrue(ResponseParser.streamRefused("'stream' is not supported with this model"))
        XCTAssertFalse(ResponseParser.streamRefused("max_tokens is too large"))
    }

    func testAnAnswerBeingWrittenShowsWithoutHalfMadeMarks() {
        XCTAssertEqual(MeetingAI.chatPartial("They decided to **ship Fri"), "They decided to **ship Fri**")
        XCTAssertEqual(MeetingAI.chatPartial("They decided to **"), "They decided to")
        XCTAssertEqual(MeetingAI.chatPartial("They decided to *"), "They decided to")
        XCTAssertEqual(MeetingAI.chatPartial("Ship Friday [1"), "Ship Friday")
        XCTAssertEqual(MeetingAI.chatPartial("Ship Friday [1, 2"), "Ship Friday")
        XCTAssertEqual(MeetingAI.chatPartial("Ship Friday ["), "Ship Friday")
        XCTAssertEqual(MeetingAI.chatPartial("Run `npm te"), "Run `npm te`")
        XCTAssertEqual(MeetingAI.chatPartial("**Done** [1] and `x`"), "**Done** [1] and `x`")
        XCTAssertEqual(MeetingAI.chatPartial("- **One** [1]\n- **Tw"), "- **One** [1]\n- **Tw**")
        XCTAssertEqual(MeetingAI.chatPartial("See [the plan"), "See [the plan", "only a number is a citation")
        XCTAssertEqual(MeetingAI.chatPartial("<think>plan</think>Ship **Fri"), "Ship **Fri**")
        XCTAssertEqual(MeetingAI.chatPartial(""), "")
        // Whatever has arrived, no stars are left for the reader
        let whole = "They decided to **ship Friday** [1].\n\n- Budget: `10k` [2]\n- Owner: **Bo**"
        XCTAssertEqual(MeetingAI.chatPartial(whole), whole)
        for end in whole.indices {
            let shown = String(MarkdownText.attributed(MeetingAI.chatPartial(String(whole[..<end]))).characters)
            XCTAssertFalse(shown.contains("**"), "stars shown at \(whole[..<end].count): \(shown)")
        }
    }
}

final class StreamingClientTests: XCTestCase {
    private let msgs = [ChatMessage(role: "system", content: "Answer from the passages."), ChatMessage(role: "user", content: "What was decided?")]

    private func ep(key: String? = "stub-key-value-1234") -> AIEndpoint {
        AIEndpoint(provider: .custom, baseURL: URL(string: "https://endpoint.example/v1"), apiKey: key, model: "stub-model",
                   contextTokens: 32_768, consentGranted: true)
    }

    private func sent() throws -> [String: Any] {
        try XCTUnwrap(try JSONSerialization.jsonObject(with: StubURLProtocol.lastBody ?? Data()) as? [String: Any])
    }

    private let stream = [
        #"data: {"choices":[{"delta":{"content":"<think>pl"}}]}"#, "",
        #"data: {"choices":[{"delta":{"content":"an</think>Ship"}}]}"#, "",
        #"data: {"choices":[{"delta":{"content":" Friday"}}]}"#, "",
        #"data: {"choices":[{"delta":{"content":"."},"finish_reason":"stop"}]}"#, "",
        "data: [DONE]", "", "",
    ].joined(separator: "\r\n")

    func testAStreamedAnswerReachesTheSinkWhileItIsWritten() async throws {
        let client = AIClient(protocolClasses: [StubURLProtocol.self])
        let body = stream
        StubURLProtocol.handler = { _ in (200, Data(body.utf8)) }
        let seen = Seen()
        let answer = try await client.complete(msgs, maxTokens: 300, endpoint: ep()) { seen.add($0) }
        XCTAssertEqual(answer, "Ship Friday.")
        XCTAssertEqual(seen.texts, ["Ship", "Ship Friday", "Ship Friday."], "the whole answer so far each time, never the thinking")
        let request = try sent()
        XCTAssertEqual(request["stream"] as? Bool, true)
        XCTAssertEqual(request["max_tokens"] as? Int, 300)
        XCTAssertNil(request["response_format"])

        // Without a sink the same endpoint is asked the plain way
        StubURLProtocol.handler = { _ in (200, Data(#"{"choices":[{"message":{"content":"Whole."},"finish_reason":"stop"}]}"#.utf8)) }
        let whole = try await client.complete(msgs, maxTokens: 300, endpoint: ep())
        XCTAssertEqual(whole, "Whole.")
        XCTAssertEqual(try sent()["stream"] as? Bool, false)
    }

    func testAServerThatCannotStreamStillAnswers() async throws {
        let client = AIClient(protocolClasses: [StubURLProtocol.self])
        // It ignores `stream` and answers with plain JSON, pretty-printed
        StubURLProtocol.handler = { _ in (200, Data("{\n  \"choices\": [{\"message\": {\"content\": \"All at once.\"}, \"finish_reason\": \"stop\"}]\n}".utf8)) }
        let seen = Seen()
        let answer = try await client.complete(msgs, maxTokens: 300, endpoint: ep()) { seen.add($0) }
        XCTAssertEqual(answer, "All at once.")
        XCTAssertEqual(seen.texts, ["All at once."])

        // It refuses `stream`: asked again without it, once
        let requests = Asked()
        StubURLProtocol.handler = { _ in
            let asked = (try? JSONSerialization.jsonObject(with: StubURLProtocol.lastBody ?? Data()) as? [String: Any]) ?? [:]
            requests.add(asked["stream"] as? Bool)
            return asked["stream"] as? Bool == true
                ? (400, Data(#"{"error":{"message":"stream=true is not supported"}}"#.utf8))
                : (200, Data(#"{"choices":[{"message":{"content":"Second try."},"finish_reason":"stop"}]}"#.utf8))
        }
        let again = Seen()
        let second = try await client.complete(msgs, maxTokens: 300, endpoint: ep()) { again.add($0) }
        XCTAssertEqual(second, "Second try.")
        XCTAssertEqual(again.texts, ["Second try."])
        XCTAssertEqual(requests.streams, [true, false])

        // An error status is the same error as ever, key redacted
        StubURLProtocol.handler = { _ in (401, Data(#"{"error":{"message":"Bad key stub-key-value-1234"}}"#.utf8)) }
        do {
            _ = try await client.complete(msgs, maxTokens: 300, endpoint: ep()) { _ in }
            XCTFail("401 must throw")
        } catch {
            guard case .wrongKey(let m) = error as? AIError else { return XCTFail("\(error)") }
            XCTAssertFalse(m.contains("stub-key-value-1234"))
        }

        // A stream that just stops is not an answer
        StubURLProtocol.handler = { _ in (200, Data("data: {\"choices\":[{\"delta\":{\"content\":\"They decided to\"}}]}\n\n".utf8)) }
        do {
            _ = try await client.complete(msgs, maxTokens: 300, endpoint: ep()) { _ in }
            XCTFail("a cut stream must throw")
        } catch {
            guard case .unreachable = error as? AIError else { return XCTFail("\(error)") }
        }
    }
}

@MainActor
final class ChatStreamTests: XCTestCase {
    func testOnlyTheQuestionWaitedOnIsShownAndItOnlyGrows() {
        let stream = ChatStream()
        let first = stream.begin()
        stream.show("They decided to **ship", for: first)
        XCTAssertEqual(stream.text, "They decided to **ship**")
        // An update that arrives late (shorter) doesn't take words back
        stream.show("They", for: first)
        XCTAssertEqual(stream.text, "They decided to **ship**")
        // The next question: words for the first land nowhere
        let second = stream.begin()
        XCTAssertEqual(stream.text, "")
        stream.show("They decided to **ship Friday**", for: first)
        XCTAssertEqual(stream.text, "")
        stream.show("Marcus", for: second)
        XCTAssertEqual(stream.text, "Marcus")
        stream.end(first)
        XCTAssertEqual(stream.text, "Marcus", "ending an earlier question leaves this one")
        stream.end(second)
        XCTAssertEqual(stream.text, "")
        stream.show("late", for: second)
        XCTAssertEqual(stream.text, "")
    }

    func testTextRemovedFromARecordingTakesTheAnswerOffTheScreen() throws {
        let container = try ModelContainer(for: Schema(Storage.modelTypes), configurations: ModelConfiguration(isStoredInMemoryOnly: true))
        let stream = ChatStream()
        let asked = stream.begin()
        stream.show("Marcus owns", for: asked)
        XCTAssertFalse(stream.purged(since: asked))
        ChatStore.purge(meetingID: UUID(), title: "Kickoff", deleted: false, context: container.mainContext)
        XCTAssertTrue(stream.purged(since: asked), "the finished answer is not kept (ChatView.ask)")
        stream.show("Marcus owns the vendor contract", for: asked)
        XCTAssertEqual(stream.text, "", "what was shown goes, and no more comes")
        stream.end(asked)
        // The next question is answered as ever
        let next = stream.begin()
        stream.show("Friday", for: next)
        XCTAssertEqual(stream.text, "Friday")
        XCTAssertFalse(stream.purged(since: next))
    }
}
