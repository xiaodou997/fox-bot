import AppKit
import CoreGraphics
import Foundation
import OCRKit
import ProbeKit

private struct NativeGateReport: Encodable {
    let schemaVersion = "foxbot.macos-send-gate.v1"
    let status: String
    let ready: Bool
    let blockers: [String]
    let frontmost: Bool
    let stableTwoReads: Bool
    let applicationSessionMatches: Bool
    let conversationResolved: Bool
    let conversationMatches: Bool
    let draftMatches: Bool
    let recentUserInput: Bool
    let composingState = "UNVERIFIED"
    let readOnly = true
    let rawTextIncluded = false
    let imageSaved = false
    let writeOperations = 0
    let sendOperations = 0
    let networkRequests = 0
}

private func emit(_ report: NativeGateReport) {
    let encoder = JSONEncoder()
    encoder.keyEncodingStrategy = .convertToSnakeCase
    encoder.outputFormatting = [.sortedKeys]
    if let data = try? encoder.encode(report) {
        FileHandle.standardOutput.write(data)
        FileHandle.standardOutput.write(Data([10]))
    }
}

private func isHex64(_ value: String) -> Bool {
    value.count == 64
        && value.allSatisfy { $0.isHexDigit && !$0.isUppercase }
        && !value.allSatisfy { $0 == "0" }
}

private func wechatIsFrontmost() -> Bool {
    NSWorkspace.shared.frontmostApplication?.bundleIdentifier == TargetApp.wechat.bundleID
}

private func recentUserInput() -> Bool {
    let types: [CGEventType] = [
        .keyDown, .flagsChanged, .leftMouseDown, .rightMouseDown, .otherMouseDown, .scrollWheel
    ]
    let quiet = types
        .map { CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: $0) }
        .min() ?? 0
    return quiet < 1.0
}

private func captureWithRetry(
    _ source: NativeWindowSource,
    attempts: Int = 3
) async throws -> WeChatComposerObservation {
    precondition((1...3).contains(attempts))
    var lastError: Error?
    for index in 0..<attempts {
        do {
            return try await WeChatComposerProbe.capture(source: source)
        } catch {
            lastError = error
            if index + 1 < attempts {
                try? await Task.sleep(nanoseconds: 200_000_000)
            }
        }
    }
    throw lastError ?? OCRFailure.focusedWindowUnavailable
}

private func blockedCaptureReport(frontmost: Bool) -> NativeGateReport {
    let facts = NativeSendGateFacts(
        captureReady: false,
        frontmost: frontmost,
        stableTwoReads: false,
        applicationSessionMatches: false,
        conversationResolved: false,
        conversationMatches: false,
        draftMatches: false,
        recentUserInput: recentUserInput(),
        composingVerifiedSafe: false
    )
    let decision = NativeSendGatePolicy.evaluate(facts)
    return NativeGateReport(
        status: "GATE_BLOCKED",
        ready: false,
        blockers: decision.blockers.map(\.rawValue),
        frontmost: facts.frontmost,
        stableTwoReads: facts.stableTwoReads,
        applicationSessionMatches: facts.applicationSessionMatches,
        conversationResolved: facts.conversationResolved,
        conversationMatches: facts.conversationMatches,
        draftMatches: facts.draftMatches,
        recentUserInput: facts.recentUserInput
    )
}

@main
struct SendGateMain {
    static func main() async {
        let application = NSApplication.shared
        application.setActivationPolicy(.prohibited)
        try? await Task.sleep(nanoseconds: 250_000_000)
        let args = Array(CommandLine.arguments.dropFirst())
        guard args.count == 4,
              args[0] == "--expected-app-session",
              isHex64(args[1]),
              args[2] == "--expected-conversation",
              isHex64(args[3])
        else {
            emit(blockedCaptureReport(frontmost: wechatIsFrontmost()))
            return
        }
        let expectedAppSession = args[1]
        let expectedConversation = args[3]
        let input = FileHandle.standardInput.readDataToEndOfFile()
        guard input.count <= 4096,
              let expectedDraft = String(data: input, encoding: .utf8),
              !expectedDraft.isEmpty,
              !expectedDraft.contains("\0")
        else {
            emit(blockedCaptureReport(frontmost: wechatIsFrontmost()))
            return
        }

        let frontmost = wechatIsFrontmost()
        guard frontmost else {
            emit(blockedCaptureReport(frontmost: false))
            return
        }

        do {
            let source = NativeWindowSource()
            let first = try await captureWithRetry(source)
            try await Task.sleep(nanoseconds: 250_000_000)
            let second = try await captureWithRetry(source)
            let firstIdentity = first.privateSnapshot
            let secondIdentity = second.privateSnapshot
            let stable =
                first.window.id == second.window.id
                && sameWindowFrame(first.window.frame, second.window.frame)
                && firstIdentity.applicationSessionFingerprint
                    == secondIdentity.applicationSessionFingerprint
                && firstIdentity.conversationFingerprint
                    == secondIdentity.conversationFingerprint
            let resolved =
                !secondIdentity.partialReasons.contains("CONVERSATION_IDENTITY_UNRESOLVED")
                && secondIdentity.conversationFingerprint
                    != WeChatMessageParser.unresolvedConversationFingerprint
            let appMatches =
                firstIdentity.applicationSessionFingerprint == expectedAppSession
                && secondIdentity.applicationSessionFingerprint == expectedAppSession
            let conversationMatches =
                firstIdentity.conversationFingerprint == expectedConversation
                && secondIdentity.conversationFingerprint == expectedConversation
            let draftMatches =
                WeChatDraftPolicy.verified(expectedDraft, snapshot: first.draftSnapshot)
                && WeChatDraftPolicy.verified(expectedDraft, snapshot: second.draftSnapshot)
            let facts = NativeSendGateFacts(
                captureReady: true,
                frontmost: frontmost,
                stableTwoReads: stable,
                applicationSessionMatches: appMatches,
                conversationResolved: resolved,
                conversationMatches: conversationMatches,
                draftMatches: draftMatches,
                recentUserInput: recentUserInput(),
                // WeChat 4.1.13 exposes no semantic editor/marked-text state through AX.
                // Until a reliable signal exists, composing must remain fail-closed.
                composingVerifiedSafe: false
            )
            let decision = NativeSendGatePolicy.evaluate(facts)
            emit(NativeGateReport(
                status: decision.ready ? "GATE_READY" : "GATE_BLOCKED",
                ready: decision.ready,
                blockers: decision.blockers.map(\.rawValue),
                frontmost: facts.frontmost,
                stableTwoReads: facts.stableTwoReads,
                applicationSessionMatches: facts.applicationSessionMatches,
                conversationResolved: facts.conversationResolved,
                conversationMatches: facts.conversationMatches,
                draftMatches: facts.draftMatches,
                recentUserInput: facts.recentUserInput
            ))
        } catch {
            emit(blockedCaptureReport(frontmost: frontmost))
        }
    }
}
