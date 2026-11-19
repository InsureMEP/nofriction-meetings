import Foundation

struct ChatMessage: Codable, Sendable, Equatable {
    var role: String   // "system" | "user" | "assistant"
    var content: String
}

/// Everything needed for one call: resolved from AISettings + Keychain on the main actor.
struct AIEndpoint: Sendable {
    let provider: AIProvider
    let baseURL: URL?
    let apiKey: String?
    let model: String
    let contextTokens: Int
    let consentGranted: Bool

    var needsConsent: Bool { URLPolicy.needsConsent(provider: provider, baseURL: baseURL) }
}

struct ModelInfo: Codable, Sendable, Hashable {
    let id: String
    let contextTokens: Int?
}

enum AIError: LocalizedError, Equatable {
    case notConfigured
    case consentRequired(String)
    case insecureURL
    case wrongKey(String)
    case noCredit(String)
    case unreachable(String)
    case atsBlocked
    case server(Int, String)
    case emptyAnswer
    case truncated
    case tooLarge
    case refused
    case badResponse
    case onDevice(String)

    var errorDescription: String? {
        switch self {
        case .notConfigured: "Configure Apple on-device or enter your AI endpoint and model in Settings first."
        case .consentRequired(let p): "Allow sending recording content to \(p) first."
        case .insecureURL: "That address isn't allowed. Cloud services need https://. Plain http:// only works for this device, your local network or a Tailscale address."
        case .wrongKey(let m): "Wrong or revoked key." + (m.isEmpty ? "" : " \(m)")
        case .noCredit(let m): "No credit left, or rate-limited." + (m.isEmpty ? "" : " \(m)")
        case .unreachable(let m): "Can't reach the AI service." + (m.isEmpty ? "" : " \(m)")
        case .atsBlocked: "iOS blocks plain http:// to that host. Use https:// (Tailscale can issue a certificate with `tailscale cert`) or the machine's local IP address."
        case .server(let s, let m): "The AI service returned an error (\(s))." + (m.isEmpty ? "" : " \(m)")
        case .emptyAnswer: "The model returned no text. Try again, or pick another model in Settings."
        case .truncated: "The model ran out of room before answering. Try again, or pick another model in Settings."
        case .tooLarge: "The AI service sent back more than 4 MB. Stopped reading."
        case .refused: "The model declined this request."
        case .badResponse: "The AI service sent a response this app couldn't read."
        case .onDevice(let m): m
        }
    }
}

// MARK: - Context fitting

enum ContextFit {
    static let defaultContextTokens = 32_768
    static let charsPerToken = 3.2
    static let reserveTokens = 512
    static let marker = "\n\n[… middle of transcript trimmed to fit the model's context …]\n\n"

    static func budgetChars(contextTokens: Int, maxTokens: Int) -> Int {
        max(0, Int(Double(contextTokens - maxTokens - reserveTokens) * charsPerToken))
    }

    /// Trim the longest message in the middle so prompt + answer fit the window.
    static func fit(_ messages: [ChatMessage], contextTokens: Int = defaultContextTokens, maxTokens: Int) -> [ChatMessage] {
        let budget = budgetChars(contextTokens: contextTokens, maxTokens: maxTokens)
        let total = messages.reduce(0) { $0 + $1.content.count }
        guard total > budget,
              let i = messages.indices.max(by: { messages[$0].content.count < messages[$1].content.count })
        else { return messages }
        var out = messages
        let text = out[i].content
        let keep = max(0, text.count - (total - budget) - marker.count)
        let head = keep * 6 / 10
        out[i].content = String(text.prefix(head)) + marker + String(text.suffix(keep - head))
        return out
    }
}

// MARK: - Request building (pure; unit-tested)

enum RequestBuilder {
    static let maxOutputTokens = 16_000

    /// Total time allowed for one completion.
    static func timeout(maxTokens: Int) -> TimeInterval { 60 + Double(maxTokens) / 8 }

    /// Allow extra output budget when the configured model uses reasoning tokens.
    static func outputBudget(proto: AIProtocol, requested: Int, completionTokens: Bool) -> Int {
        switch proto {
        case .openai: completionTokens ? min(requested + 4096, maxOutputTokens) : requested
        case .foundationModels: requested
        }
    }

    static func chat(_ ep: AIEndpoint, messages: [ChatMessage], maxTokens: Int, temperature: Double?,
                     completionTokens: Bool = false, stream: Bool = false) throws -> URLRequest {
        guard ep.provider == .custom, let base = ep.baseURL,
              !ep.model.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw AIError.notConfigured }
        guard URLPolicy.isAllowed(base) else { throw AIError.insecureURL }
        switch ep.provider.proto {
        case .openai:
            var req = URLRequest(url: base.appending(path: "chat/completions"))
            req.httpMethod = "POST"
            req.setValue("application/json", forHTTPHeaderField: "Content-Type")
            if let key = ep.apiKey, !key.isEmpty { req.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization") }
            var body: [String: Any] = [
                "model": ep.model,
                "messages": messages.map { ["role": $0.role, "content": $0.content] },
                "stream": stream,
            ]
            if completionTokens {
                body["max_completion_tokens"] = maxTokens
            } else {
                body["max_tokens"] = maxTokens
                if let temperature { body["temperature"] = temperature }
            }
            req.httpBody = try JSONSerialization.data(withJSONObject: body, options: [.sortedKeys])
            req.timeoutInterval = timeout(maxTokens: maxTokens)
            return req
        case .foundationModels:
            throw AIError.badResponse
        }
    }

    /// The connection test body: the single word "Hi" with a one-token answer.
    /// Never any recording content, so it needs no consent.
    static func probe(_ ep: AIEndpoint, completionTokens: Bool = false) throws -> URLRequest {
        var req = try chat(ep, messages: [ChatMessage(role: "user", content: "Hi")], maxTokens: 1,
                           temperature: nil, completionTokens: completionTokens)
        req.timeoutInterval = 30
        return req
    }

}

// MARK: - Connection test (explicit tap; one fixed word, never recording content)

enum ConnectionTest {
    enum Outcome: Equatable, Sendable {
        case connected(String)
        case wrongKey(String)
        case noCredit(String)
        case unreachable(String)
        case badURL(String)
        case modelMissing(String)
        case noKey(String)
        case other(String)

        var ok: Bool { if case .connected = self { return true } else { return false } }

        var message: String {
            switch self {
            case .connected(let m), .wrongKey(let m), .noCredit(let m), .unreachable(let m),
                 .badURL(let m), .modelMissing(let m), .noKey(let m), .other(let m): m
            }
        }
    }

    /// Plain words from the probe's status and body. Pure; unit tested.
    static func classify(host: String, model: String, status: Int, data: Data, secrets: [String]) -> Outcome {
        let msg = ResponseParser.errorMessage(data, secrets: secrets)
        let mentionsModel = msg.lowercased().contains("model")
        switch status {
        case 200..<300: return .connected("Connected to \(host) with model \(model).")
        case 401, 403: return .wrongKey("\(host) rejected the API key (\(status)). Check the key and save it again.")
        case 402, 429: return .noCredit("\(host) accepted the key but reported no credit or a rate limit (\(status)). \(msg)")
        case 300..<400: return .badURL("\(host) redirected the request (\(status)); the base URL is probably wrong.")
        case 404 where !mentionsModel: return .badURL("\(host) has no chat/completions at this base URL (404). It usually ends in /v1.")
        case 400, 404, 422: return mentionsModel
            ? .modelMissing("\(host) doesn't offer the model '\(model)'. \(msg)")
            : .other("\(host) answered \(status): \(msg)")
        default: return .other("\(host) answered \(status): \(msg)")
        }
    }

    /// Transport and configuration errors in the same plain words.
    static func outcome(for error: AIError, host: String) -> Outcome {
        switch error {
        case .notConfigured: return .badURL("Enter a base URL and model, then save the connection first.")
        case .insecureURL: return .badURL(error.localizedDescription)
        case .atsBlocked: return .unreachable(error.localizedDescription)
        case .unreachable(let m): return .unreachable("Couldn't reach \(host). \(m)")
        case .wrongKey(let m): return .wrongKey("\(host) rejected the API key. \(m)")
        case .noCredit(let m): return .noCredit("\(host) reported no credit or a rate limit. \(m)")
        case .tooLarge, .badResponse: return .other("\(host) sent a response this app couldn't read.")
        default: return .other(error.localizedDescription)
        }
    }
}

// MARK: - Response parsing (pure; unit-tested)

enum ResponseParser {
    /// Best-effort human message from an error body, redacted and short.
    static func errorMessage(_ data: Data, secrets: [String]) -> String {
        var msg = ""
        let obj = try? JSONSerialization.jsonObject(with: data)
        let root = (obj as? [String: Any]) ?? ((obj as? [Any])?.first as? [String: Any])
        if let err = root?["error"] as? [String: Any], let m = err["message"] as? String {
            msg = m
        } else if let m = root?["error"] as? String {
            msg = m
        } else if let m = root?["message"] as? String {
            msg = m
        } else if let m = root?["detail"] as? String {
            msg = m
        } else if obj == nil {
            msg = String(decoding: data.prefix(200), as: UTF8.self)
        }
        msg = msg.trimmingCharacters(in: .whitespacesAndNewlines)
        if msg.count > 200 { msg = String(msg.prefix(200)) + "…" }
        return Redactor.redact(msg, secrets: secrets)
    }

    static func statusError(_ status: Int, _ data: Data, secrets: [String]) -> AIError {
        let m = errorMessage(data, secrets: secrets)
        switch status {
        case 401, 403: return .wrongKey(m)
        case 402, 429: return .noCredit(m)
        case 400 where m.lowercased().contains("api key"): return .wrongKey(m)
        default: return .server(status, m)
        }
    }

    /// A 400 that says this model wants max_completion_tokens / default temperature.
    static func wantsCompletionTokens(_ message: String) -> Bool {
        let m = message.lowercased()
        return m.contains("max_completion_tokens") || m.contains("max_tokens")
            || (m.contains("temperature") && (m.contains("unsupported") || m.contains("not support") || m.contains("default")))
    }

    static func openAIText(_ data: Data) throws -> String {
        guard let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any],
              let choice = (obj["choices"] as? [[String: Any]])?.first else { throw AIError.badResponse }
        let message = choice["message"] as? [String: Any]
        var text = ""
        if let s = message?["content"] as? String {
            text = s
        } else if let parts = message?["content"] as? [[String: Any]] {
            text = parts.compactMap { $0["text"] as? String }.joined()
        }
        text = stripThinking(text).trimmingCharacters(in: .whitespacesAndNewlines)
        if text.isEmpty {
            if (choice["finish_reason"] as? String) == "length" { throw AIError.truncated }
            if message?["refusal"] is String { throw AIError.refused }
            throw AIError.emptyAnswer
        }
        return text
    }

    /// A 400 that names `stream`: this server can't stream. Ask again the plain way.
    static func streamRefused(_ message: String) -> Bool {
        message.lowercased().contains("stream")
    }

    /// The part of an answer still being written that can be shown: think
    /// blocks are gone (an open one hides what follows), and so is the
    /// start of a tag that hasn't finished arriving ("<thi").
    static func visibleSoFar(_ raw: String) -> String {
        var out = stripThinking(raw)
        if let open = out.lastIndex(of: "<"), "<think>".hasPrefix(out[open...]) {
            out.removeSubrange(open...)
        }
        return out.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The text of a streamed answer (server-sent events carrying
    /// OpenAI-compatible chunks), fed one line at a time. Pure; unit-tested.
    /// Reasoning deltas are not part of the answer and are skipped.
    struct StreamedText {
        /// The answer so far, as sent (think blocks still in it)
        private(set) var text = ""
        /// `finish_reason`, once the server gave one
        private(set) var finish: String?
        /// The server said the stream is complete (`[DONE]`)
        private(set) var done = false
        /// An error event inside a 200 stream: the status the same refusal
        /// would have had, and its body
        private(set) var failure: (status: Int, body: Data)?

        /// Take the next line. True when the answer grew.
        mutating func line(_ raw: String) -> Bool {
            // `event:` names, `:` keep-alive comments and blank lines carry no text
            guard raw.hasPrefix("data:") else { return false }
            let data = raw.dropFirst(5).trimmingCharacters(in: .whitespaces)
            if data == "[DONE]" { done = true; return false }
            guard let obj = try? JSONSerialization.jsonObject(with: Data(data.utf8)) as? [String: Any] else { return false }
            if let error = obj["error"] {
                let detail = error as? [String: Any]
                let kind = [detail?["type"] as? String, detail?["code"] as? String].compactMap { $0 }.joined(separator: " ").lowercased()
                let status = ["authentication", "invalid_api_key", "permission"].contains(where: kind.contains) ? 401
                    : ["rate_limit", "quota", "credit", "billing"].contains(where: kind.contains) ? 429 : 500
                failure = (status, Data(data.utf8))
                return false
            }
            guard let choice = (obj["choices"] as? [[String: Any]])?.first else { return false }
            let before = text.count
            let delta = choice["delta"] as? [String: Any]
            if let s = delta?["content"] as? String {
                text += s
            } else if let parts = delta?["content"] as? [[String: Any]] {
                text += parts.compactMap { $0["text"] as? String }.joined()
            }
            if let f = choice["finish_reason"] as? String { finish = f }
            return text.count > before
        }

        /// The finished answer, by the same rules as a whole response. A
        /// body that just stopped (no finish reason, no end marker) is a
        /// broken connection: half an answer is never returned as a whole one.
        func result(secrets: [String]) throws -> String {
            if let failure { throw ResponseParser.statusError(failure.status, failure.body, secrets: secrets) }
            guard done || finish != nil else { throw AIError.unreachable("The connection closed before the answer was finished.") }
            let answer = ResponseParser.stripThinking(text).trimmingCharacters(in: .whitespacesAndNewlines)
            if answer.isEmpty { throw finish == "length" ? AIError.truncated : AIError.emptyAnswer }
            return answer
        }
    }

    /// Some compatible models inline reasoning in <think>…</think>.
    static func stripThinking(_ s: String) -> String {
        guard s.contains("<think>") else { return s }
        var out = s
        while let open = out.range(of: "<think>") {
            if let close = out.range(of: "</think>", range: open.upperBound..<out.endIndex) {
                out.removeSubrange(open.lowerBound..<close.upperBound)
            } else {
                out.removeSubrange(open.lowerBound..<out.endIndex)
            }
        }
        return out
    }

}

// MARK: - Client

/// One place every AI request goes through, with the guardrails from the spec:
/// max_tokens always, fitted context, no response_format, timeouts, HTTPS for
/// cloud, redacted errors, a 4 MB response cap and no redirects.
actor AIClient {
    static let shared = AIClient()
    static let maxResponseBytes = 4 * 1024 * 1024
    private static let completionTokenModelsKey = "ai.completionTokenModels"

    /// Models known to want max_completion_tokens and no temperature ("endpoint|model").
    private var completionTokenModels: Set<String>
    /// URLProtocol classes for tests (a stub answers instead of the network). Empty in the app.
    private let protocolClasses: [AnyClass]

    init(protocolClasses: [AnyClass] = []) {
        completionTokenModels = Set(UserDefaults.standard.stringArray(forKey: Self.completionTokenModelsKey) ?? [])
        self.protocolClasses = protocolClasses
    }

    /// The connection test: one fixed word with a one-token answer to the saved
    /// endpoint with its saved key. It carries no recording content, so it is
    /// the only request allowed before consent, and it runs only on an
    /// explicit tap (never at launch or on save). Failures come back as plain
    /// words, never thrown.
    func testConnection(_ ep: AIEndpoint) async -> ConnectionTest.Outcome {
        if ep.provider.proto == .foundationModels {
            return AppleOnDevice.isAvailable ? .connected("Apple on-device model is available.") : .unreachable("Apple on-device model is not available on this device.")
        }
        guard ep.provider == .custom, let base = ep.baseURL,
              !ep.model.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            return ConnectionTest.outcome(for: .notConfigured, host: ep.provider.name)
        }
        let host = base.host() ?? base.absoluteString
        guard URLPolicy.isAllowed(base) else { return ConnectionTest.outcome(for: .insecureURL, host: host) }
        let secrets = [ep.apiKey].compactMap { $0 }
        var useCompletionTokens = completionTokenModels.contains("\(base.absoluteString)|\(ep.model)")
        for attempt in 0..<2 {
            do {
                let req = try RequestBuilder.probe(ep, completionTokens: useCompletionTokens)
                let (data, status) = try await send(req, secrets: secrets)
                if status == 400, attempt == 0, !useCompletionTokens,
                   ResponseParser.wantsCompletionTokens(ResponseParser.errorMessage(data, secrets: secrets)) {
                    useCompletionTokens = true
                    continue
                }
                return ConnectionTest.classify(host: host, model: ep.model, status: status, data: data, secrets: secrets)
            } catch let e as AIError {
                return ConnectionTest.outcome(for: e, host: host)
            } catch {
                return .unreachable("Couldn't reach \(host). \(Redactor.redact(error.localizedDescription, secrets: secrets))")
            }
        }
        return .other("\(host) rejected the request twice.")
    }

    /// Where a streamed answer goes while it is written: called with the
    /// whole answer so far (never a fragment), each time it grows.
    typealias Partial = @Sendable (String) -> Void

    /// One completion. With `onText`, the same request asks for a stream and
    /// the answer is handed over while it is written (Chat); the result is
    /// the same whole answer. A server that answers with plain JSON is read
    /// whole, and one that refuses `stream` is asked again without it.
    func complete(_ messages: [ChatMessage], maxTokens: Int, temperature: Double = 0.3, endpoint ep: AIEndpoint,
                  onText: Partial? = nil) async throws -> String {
        if ep.needsConsent && !ep.consentGranted { throw AIError.consentRequired(ep.baseURL?.host() ?? ep.provider.name) }
        let requested = min(max(maxTokens, 16), 8192)

        if ep.provider.proto == .foundationModels {
            return try await AppleOnDevice.complete(messages, maxTokens: requested, temperature: temperature, onText: onText)
        }
        guard ep.provider == .custom, let base = ep.baseURL,
              !ep.model.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { throw AIError.notConfigured }
        guard URLPolicy.isAllowed(base) else { throw AIError.insecureURL }

        let secrets = [ep.apiKey].compactMap { $0 }
        let modelKey = "\(base.absoluteString)|\(ep.model)"
        var useCompletionTokens = completionTokenModels.contains(modelKey)
        var streaming = onText != nil
        // The first request, and one more for each thing a server can refuse
        // (the token parameter, streaming)
        for _ in 0..<3 {
            let budget = RequestBuilder.outputBudget(proto: ep.provider.proto, requested: requested,
                                                     completionTokens: useCompletionTokens)
            let fitted = ContextFit.fit(messages, contextTokens: ep.contextTokens, maxTokens: budget)
            let req = try RequestBuilder.chat(ep, messages: fitted, maxTokens: budget, temperature: temperature,
                                              completionTokens: useCompletionTokens, stream: streaming)
            let data: Data
            let status: Int
            if streaming, let onText {
                switch try await sendStreaming(req, secrets: secrets, onText: onText) {
                case .text(let answer): return answer
                case .whole(let d, let s): (data, status) = (d, s)
                }
            } else {
                (data, status) = try await send(req, secrets: secrets)
            }
            if (200..<300).contains(status) {
                let answer = try ResponseParser.openAIText(data)
                // Asked to stream, answered whole: the caller still gets the answer
                onText?(answer)
                return answer
            }
            let message = ResponseParser.errorMessage(data, secrets: secrets)
            if status == 400, ep.provider.proto == .openai, !useCompletionTokens,
               ResponseParser.wantsCompletionTokens(message) {
                useCompletionTokens = true
                completionTokenModels.insert(modelKey)
                UserDefaults.standard.set(Array(completionTokenModels), forKey: Self.completionTokenModelsKey)
                continue
            }
            if status == 400, streaming, ResponseParser.streamRefused(message) {
                streaming = false
                continue
            }
            throw ResponseParser.statusError(status, data, secrets: secrets)
        }
        throw AIError.badResponse
    }

    // MARK: Transport

    private func session(for req: URLRequest, total: TimeInterval? = nil) -> URLSession {
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = req.timeoutInterval
        config.timeoutIntervalForResource = total ?? req.timeoutInterval
        config.waitsForConnectivity = false
        config.httpCookieStorage = nil
        config.urlCache = nil
        if !protocolClasses.isEmpty { config.protocolClasses = protocolClasses }
        return URLSession(configuration: config, delegate: NoRedirects(), delegateQueue: nil)
    }

    /// A transport failure as the app's own error, keys redacted.
    private func transportError(_ error: Error, secrets: [String]) -> Error {
        if let e = error as? AIError { return e }
        if error is CancellationError { return CancellationError() }
        if let e = error as? URLError {
            if e.code == .appTransportSecurityRequiresSecureConnection { return AIError.atsBlocked }
            if e.code == .cancelled { return CancellationError() }
            return AIError.unreachable(Redactor.redact(e.localizedDescription, secrets: secrets))
        }
        return AIError.unreachable(Redactor.redact(error.localizedDescription, secrets: secrets))
    }

    private func send(_ req: URLRequest, secrets: [String], total: TimeInterval? = nil) async throws -> (Data, Int) {
        let session = session(for: req, total: total)
        defer { session.finishTasksAndInvalidate() }
        do {
            let (bytes, resp) = try await session.bytes(for: req)
            guard let http = resp as? HTTPURLResponse else { throw AIError.badResponse }
            return (try await Self.collect(bytes, http), http.statusCode)
        } catch {
            throw transportError(error, secrets: secrets)
        }
    }

    /// The whole body, up to the cap.
    private static func collect(_ bytes: URLSession.AsyncBytes, _ http: HTTPURLResponse) async throws -> Data {
        if http.expectedContentLength > Self.maxResponseBytes { bytes.task.cancel(); throw AIError.tooLarge }
        var data = Data()
        data.reserveCapacity(Int(max(0, min(http.expectedContentLength, Int64(Self.maxResponseBytes)))))
        for try await byte in bytes {
            data.append(byte)
            if data.count > Self.maxResponseBytes { bytes.task.cancel(); throw AIError.tooLarge }
        }
        return data
    }

    /// What came back from a request that asked for a stream.
    private enum Streamed {
        /// The answer, already handed over piece by piece
        case text(String)
        /// Not a stream (an error status, or a server that answered with
        /// plain JSON): the whole body and the status, for the usual handling
        case whole(Data, Int)
    }

    /// Send a request that asks for a stream and read it line by line. A
    /// body that starts with `{` is plain JSON whatever was asked for (every
    /// event line starts with a field name or `:`), so it is handed back whole.
    private func sendStreaming(_ req: URLRequest, secrets: [String], onText: Partial) async throws -> Streamed {
        let session = session(for: req)
        defer { session.finishTasksAndInvalidate() }
        do {
            let (bytes, resp) = try await session.bytes(for: req)
            guard let http = resp as? HTTPURLResponse else { throw AIError.badResponse }
            if !(200..<300).contains(http.statusCode) {
                return .whole(try await Self.collect(bytes, http), http.statusCode)
            }
            var parser = ResponseParser.StreamedText()
            var whole: Data?
            var sniffed = false
            var received = 0
            var shown = ""
            for try await line in bytes.lines {
                received += line.utf8.count + 1
                if received > Self.maxResponseBytes { bytes.task.cancel(); throw AIError.tooLarge }
                if !sniffed {
                    let first = line.trimmingCharacters(in: .whitespaces)
                    if first.isEmpty { continue }
                    sniffed = true
                    if first.hasPrefix("{") { whole = Data() }
                }
                if whole != nil {
                    whole?.append(contentsOf: line.utf8)
                    whole?.append(0x0A)
                    continue
                }
                if parser.line(line) {
                    let now = ResponseParser.visibleSoFar(parser.text)
                    if now != shown {
                        shown = now
                        onText(now)
                    }
                }
                if parser.done || parser.failure != nil {
                    bytes.task.cancel()
                    break
                }
            }
            if let whole { return .whole(whole, http.statusCode) }
            let answer = try parser.result(secrets: secrets)
            if answer != shown { onText(answer) }
            return .text(answer)
        } catch {
            throw transportError(error, secrets: secrets)
        }
    }
}

/// Keys must never follow a redirect to another host.
private final class NoRedirects: NSObject, URLSessionTaskDelegate, Sendable {
    func urlSession(_: URLSession, task _: URLSessionTask, willPerformHTTPRedirection _: HTTPURLResponse,
                    newRequest _: URLRequest) async -> URLRequest? { nil }
}
