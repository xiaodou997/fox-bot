import AppKit
import CoreGraphics
import Foundation
import OCRKit
import ProbeKit

private struct Request: Decodable {
    let id: UInt64
    let command: String
    let expected: SendObservation?
    let text: String?
    let actionId: String?
}

private struct Reply: Encodable {
    let schemaVersion = "foxbot.native-send-worker.v3"
    let id: UInt64
    var status: String
    var observation: SendObservation?
    var writeAttempted = false
    var sendAttempted = false
    var verifiedOutgoing = false
}

private enum Failure: String, Error {
    case invalidRequest = "INVALID_REQUEST"
    case appNotFrontmost = "APP_NOT_FRONTMOST"
    case captureFailed = "CAPTURE_FAILED"
    case targetChanged = "TARGET_CHANGED"
    case draftNotEmpty = "DRAFT_NOT_EMPTY"
    case draftMismatch = "DRAFT_MISMATCH"
    case receiptRevisionMismatch = "RECEIPT_REVISION_MISMATCH"
    case sendButtonUnavailable = "SEND_BUTTON_UNAVAILABLE"
    case actionAlreadyAttempted = "ACTION_ALREADY_ATTEMPTED"
    case ownerLost = "OWNER_LOST"
    case inputFailed = "INPUT_FAILED"
}

private func frontmost() -> Bool {
    NSWorkspace.shared.frontmostApplication?.bundleIdentifier == TargetApp.wechat.bundleID
}

private func emit(_ reply: Reply) {
    let encoder = JSONEncoder()
    encoder.keyEncodingStrategy = .convertToSnakeCase
    encoder.outputFormatting = [.sortedKeys]
    guard let data = try? encoder.encode(reply) else { return }
    FileHandle.standardOutput.write(data)
    FileHandle.standardOutput.write(Data([10]))
}

private func click(_ point: CGPoint) throws {
    guard let down = CGEvent(mouseEventSource: nil, mouseType: .leftMouseDown,
                             mouseCursorPosition: point, mouseButton: .left),
          let up = CGEvent(mouseEventSource: nil, mouseType: .leftMouseUp,
                           mouseCursorPosition: point, mouseButton: .left) else { throw Failure.inputFailed }
    down.post(tap: .cghidEventTap)
    up.post(tap: .cghidEventTap)
}

private func input(_ text: String) throws {
    // Bounded Unicode packets; never synthesize Return, tabs, or IME composition.
    var packets: [[UniChar]] = [[]]
    for character in text {
        let units = Array(String(character).utf16)
        guard units.count <= 20 else { throw Failure.invalidRequest }
        if packets[packets.count - 1].count + units.count > 20 { packets.append([]) }
        packets[packets.count - 1].append(contentsOf: units)
    }
    for units in packets where !units.isEmpty {
        guard let down = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: true),
              let up = CGEvent(keyboardEventSource: nil, virtualKey: 0, keyDown: false)
        else { throw Failure.inputFailed }
        units.withUnsafeBufferPointer { buffer in
            if let base = buffer.baseAddress {
                down.keyboardSetUnicodeString(stringLength: units.count, unicodeString: base)
                up.keyboardSetUnicodeString(stringLength: units.count, unicodeString: base)
            }
        }
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
    }
}

@main
struct SendMain {
    static func main() async {
        let args = Array(CommandLine.arguments.dropFirst())
        guard args == ["--worker"] || args == ["--worker", "--allow-single-send"] else { return }
        let allowWrite = args.count == 2
        let parent = getppid()
        let app = NSApplication.shared
        app.setActivationPolicy(.prohibited)
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let source = NativeWindowSource()
        var previousId: UInt64 = 0
        var filledAction: String?
        var filledText: String?
        var fillUsed = false
        var sendUsed = false

        func checkOwner() throws {
            guard parent > 1, getppid() == parent else { throw Failure.ownerLost }
            guard frontmost() else { throw Failure.appNotFrontmost }
        }
        func capture() async throws -> (WeChatComposerObservation, SendObservation) {
            try checkOwner()
            for attempt in 0..<3 {
                do {
                    let raw = try await WeChatComposerProbe.capture(source: source, includeSendControl: true)
                    let snapshot = WeChatSendPolicy.observation(raw, frontmost: frontmost())
                    guard snapshot.frontmost, snapshot.conversationResolved else { throw Failure.targetChanged }
                    return (raw, snapshot)
                } catch {
                    if attempt == 2 { throw Failure.captureFailed }
                    try await Task.sleep(nanoseconds: 200_000_000)
                }
            }
            throw Failure.captureFailed
        }

        while let line = readLine() {
            guard line.utf8.count <= 65_536,
                  let request = try? decoder.decode(Request.self, from: Data(line.utf8)),
                  request.id > previousId else { break }
            previousId = request.id
            var reply = Reply(id: request.id, status: "INVALID_REQUEST")
            do {
                if request.command == "warmup" {
                    reply.status = VisionOCR.warmup().succeeded ? "WARMED" : "WARMUP_FAILED"
                } else if request.command == "inspect" {
                    let (_, snapshot) = try await capture()
                    reply.observation = snapshot
                    reply.status = "OBSERVED"
                } else if ["fill", "send", "reconcile"].contains(request.command) {
                    guard let expected = request.expected, let text = request.text,
                          let action = request.actionId, !action.isEmpty, action.utf8.count <= 128,
                          WeChatSendPolicy.supportedText(text) else { throw Failure.invalidRequest }
                    let (raw, current) = try await capture()
                    reply.observation = current
                    guard expected.sameSurface(as: current) else { throw Failure.targetChanged }
                    guard expected.evidenceRevision == WeChatSendPolicy.evidenceRevision,
                          current.evidenceRevision == WeChatSendPolicy.evidenceRevision
                    else { throw Failure.receiptRevisionMismatch }
                    if request.command == "reconcile" {
                        reply.verifiedOutgoing = WeChatSendPolicy.verifiedOutgoing(before: expected, after: current, text: text)
                        reply.status = reply.verifiedOutgoing ? "VERIFIED_OUTGOING" : "UNKNOWN"
                    } else if request.command == "fill" {
                        guard allowWrite else { throw Failure.invalidRequest }
                        guard !fillUsed else { throw Failure.actionAlreadyAttempted }
                        guard expected.sameMessages(as: current) else { throw Failure.targetChanged }
                        guard current.draftState == .emptyHeuristic else { throw Failure.draftNotEmpty }
                        fillUsed = true
                        try checkOwner()
                        reply.writeAttempted = true
                        try click(CGPoint(x: raw.window.frame.minX + raw.window.frame.width * WeChatDraftPolicy.focusPoint.x,
                                          y: raw.window.frame.minY + raw.window.frame.height * WeChatDraftPolicy.focusPoint.y))
                        try await Task.sleep(nanoseconds: 150_000_000)
                        try checkOwner()
                        try input(text)
                        try await Task.sleep(nanoseconds: 600_000_000)
                        let (_, after) = try await capture()
                        reply.observation = after
                        guard expected.sameSurface(as: after), expected.sameMessages(as: after)
                        else { throw Failure.targetChanged }
                        guard after.draftText == text else { throw Failure.draftMismatch }
                        filledAction = action
                        filledText = text
                        reply.status = "FILLED"
                    } else {
                        guard allowWrite, filledAction == action, filledText == text else { throw Failure.invalidRequest }
                        guard !sendUsed else { throw Failure.actionAlreadyAttempted }
                        guard expected.sameMessages(as: current) else { throw Failure.targetChanged }
                        guard current.draftText == text else { throw Failure.draftMismatch }
                        guard let button = current.sendButton else { throw Failure.sendButtonUnavailable }
                        try checkOwner()
                        sendUsed = true
                        reply.sendAttempted = true
                        try click(CGPoint(x: raw.window.frame.minX + raw.window.frame.width * button.x,
                                          y: raw.window.frame.minY + raw.window.frame.height * button.y))
                        reply.status = "UNKNOWN"
                        for _ in 0..<6 {
                            try await Task.sleep(nanoseconds: 500_000_000)
                            let (_, after) = try await capture()
                            reply.observation = after
                            if WeChatSendPolicy.verifiedOutgoing(before: current, after: after, text: text) {
                                reply.verifiedOutgoing = true
                                reply.status = "VERIFIED_OUTGOING"
                                break
                            }
                        }
                    }
                }
            } catch let error as Failure {
                reply.status = error.rawValue
            } catch {
                reply.status = "NATIVE_OPERATION_FAILED"
            }
            emit(reply)
        }
    }
}
