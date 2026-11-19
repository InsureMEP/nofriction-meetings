// Apple on-device model (Foundation Models, macOS 26+ with Apple
// Intelligence on). Text only. Compiled into both flavors; the framework is
// weak-linked (build.rs) so the app still launches on macOS 13–15, where
// availability reports "requires_macos_26".

import Foundation
#if canImport(FoundationModels)
import FoundationModels
#endif

func nfAppleAvailability() -> (Bool, String) {
    #if canImport(FoundationModels)
    if #available(macOS 26.0, *) {
        switch SystemLanguageModel.default.availability {
        case .available:
            return (true, "")
        case .unavailable(let reason):
            switch reason {
            case .deviceNotEligible:
                return (false, "device_not_eligible")
            case .appleIntelligenceNotEnabled:
                return (false, "apple_intelligence_off")
            case .modelNotReady:
                return (false, "model_not_ready")
            @unknown default:
                return (false, "unavailable")
            }
        @unknown default:
            return (false, "unavailable")
        }
    }
    return (false, "requires_macos_26")
    #else
    return (false, "sdk_without_foundation_models")
    #endif
}

/// `{"available": bool, "reason": String}` (malloc'd; free with nf_free)
@_cdecl("nf_apple_model_availability")
public func nf_apple_model_availability() -> UnsafeMutablePointer<CChar>? {
    let (ok, reason) = nfAppleAvailability()
    return nfCString(["available": ok, "reason": reason])
}

/// One-shot generation. Result: `{"ok":true,"text":…}` or `{"ok":false,"error":…}`.
/// `temperature < 0` means "model default".
@_cdecl("nf_apple_generate")
public func nf_apple_generate(
    _ instructions: UnsafePointer<CChar>,
    _ prompt: UnsafePointer<CChar>,
    _ maxTokens: Int32,
    _ temperature: Double,
    _ ctx: UnsafeMutableRawPointer?,
    _ cb: NFCallback
) {
    let context = NFContext(ptr: ctx, cb: cb)
    let instructionsText = String(cString: instructions)
    let promptText = String(cString: prompt)
    #if canImport(FoundationModels)
    if #available(macOS 26.0, *) {
        Task.detached {
            do {
                let session = instructionsText.isEmpty
                    ? LanguageModelSession()
                    : LanguageModelSession(instructions: instructionsText)
                let options = GenerationOptions(
                    temperature: temperature >= 0 ? temperature : nil,
                    maximumResponseTokens: maxTokens > 0 ? Int(maxTokens) : nil
                )
                let response = try await session.respond(to: promptText, options: options)
                context.send(["ok": true, "text": response.content])
            } catch {
                context.fail(error)
            }
        }
        return
    }
    #endif
    context.send(["ok": false, "error": nfAppleAvailability().1])
}

/// The same generation, streamed: `partial` gets the answer so far (a C
/// string) each time the model has written more, then `done` gets the JSON
/// result exactly once, last. `done` owns the context: nothing is sent
/// after it.
@_cdecl("nf_apple_generate_stream")
public func nf_apple_generate_stream(
    _ instructions: UnsafePointer<CChar>,
    _ prompt: UnsafePointer<CChar>,
    _ maxTokens: Int32,
    _ temperature: Double,
    _ ctx: UnsafeMutableRawPointer?,
    _ partial: NFCallback,
    _ done: NFCallback
) {
    let context = NFContext(ptr: ctx, cb: done)
    let progress = NFContext(ptr: ctx, cb: partial)
    let instructionsText = String(cString: instructions)
    let promptText = String(cString: prompt)
    #if canImport(FoundationModels)
    if #available(macOS 26.0, *) {
        Task.detached {
            do {
                let session = instructionsText.isEmpty
                    ? LanguageModelSession()
                    : LanguageModelSession(instructions: instructionsText)
                let options = GenerationOptions(
                    temperature: temperature >= 0 ? temperature : nil,
                    maximumResponseTokens: maxTokens > 0 ? Int(maxTokens) : nil
                )
                var soFar = ""
                for try await snapshot in session.streamResponse(to: promptText, options: options) {
                    let text = snapshot.content
                    if text != soFar {
                        soFar = text
                        text.withCString { progress.cb(progress.ptr, $0) }
                    }
                }
                context.send(["ok": true, "text": soFar])
            } catch {
                context.fail(error)
            }
        }
        return
    }
    #endif
    context.send(["ok": false, "error": nfAppleAvailability().1])
}
