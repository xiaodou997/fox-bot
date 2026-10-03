import CoreGraphics
import CryptoKit
import Foundation

public struct SendPoint: Codable, Equatable {
    public let x: Double
    public let y: Double
}

public struct SendMessageSignature: Codable, Equatable {
    public let digest: String
    public let direction: String
    public let complete: Bool
    public let continuityDigest: String?

    public func sameContext(as other: SendMessageSignature) -> Bool {
        continuityDigest != nil && continuityDigest == other.continuityDigest
            && direction == other.direction && complete == other.complete
    }
}

/// Private worker IPC. Only the draft text is plaintext; chat history is hashed.
public struct SendObservation: Codable, Equatable {
    public let applicationSession: String
    public let conversation: String
    public let windowRef: String
    public let layoutRef: String
    public let frontmost: Bool
    public let conversationResolved: Bool
    public let draftState: DraftReadState
    public let draftText: String?
    public let messages: [SendMessageSignature]
    public let sendButton: SendPoint?
    /// Missing in legacy receipts. Never infer or retrofit their evidence revision.
    public var evidenceRevision: String?

    public func sameMessages(as other: SendObservation) -> Bool {
        messages.count == other.messages.count
            && zip(messages, other.messages).allSatisfy { $0.sameContext(as: $1) }
    }

    public func sameSurface(as other: SendObservation) -> Bool {
        applicationSession == other.applicationSession && conversation == other.conversation
            && windowRef == other.windowRef && layoutRef == other.layoutRef
            && conversationResolved && other.conversationResolved
    }

    public func withDraftText(_ text: String) -> SendObservation {
        SendObservation(
            applicationSession: applicationSession, conversation: conversation,
            windowRef: windowRef, layoutRef: layoutRef, frontmost: frontmost,
            conversationResolved: conversationResolved, draftState: .nonempty,
            draftText: text, messages: messages, sendButton: sendButton,
            evidenceRevision: evidenceRevision
        )
    }
}

/// Plaintext is available only to the host's explicitly requested private read command.
public struct NativeReadMessage: Encodable {
    public let text: String
    public let direction: String
    public let complete: Bool
}

public enum WeChatSendPolicy {
    public static let evidenceRevision = "WECHAT_RECEIPT_V3"
    public static let controlRegion = CGRect(x: 0.86, y: 0.80, width: 0.14, height: 0.20)

    public static func digest(_ value: String) -> String {
        SHA256.hash(data: Data(value.utf8))
            .map { String(format: "%02x", $0) }.joined()
    }

    /// Match G2's historical-context normalization only. Exact outgoing/draft comparison
    /// still uses the unmodified text digest; never repair digits, case, punctuation or words.
    public static func continuityText(_ text: String) -> String {
        let collapsed = text.replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
            .trimmingCharacters(in: .whitespacesAndNewlines)
            .replacingOccurrences(of: #"[ \t]+"#, with: " ", options: .regularExpression)
        return collapsed.replacingOccurrences(
            of: #"(?<=[\u3400-\u4DBF\u4E00-\u9FFF\uF900-\uFAFF]) (?=[A-Za-z0-9])|(?<=[A-Za-z0-9]) (?=[\u3400-\u4DBF\u4E00-\u9FFF\uF900-\uFAFF])"#,
            with: "", options: .regularExpression)
    }

    /// G3c-1 deliberately supports short, single-line plain text only. Reject before input.
    public static func supportedText(_ text: String) -> Bool {
        !text.isEmpty && text.utf16.count <= 80
            && text == text.trimmingCharacters(in: .whitespacesAndNewlines)
            && !text.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) }
            && !text.hasSuffix("|")
    }

    public static func sendButton(_ snapshot: OCRSnapshot?) -> SendPoint? {
        guard let snapshot, snapshot.statistics.completeRecognition else { return nil }
        let candidates = snapshot.lines.filter {
            $0.confidence >= 0.8
                && ["发送", "Send"].contains($0.text.trimmingCharacters(in: .whitespacesAndNewlines))
                && controlRegion.contains(CGPoint(x: $0.bounds.midX, y: $0.bounds.midY))
        }
        guard candidates.count == 1 else { return nil }
        return SendPoint(x: Double(candidates[0].bounds.midX), y: Double(candidates[0].bounds.midY))
    }

    private static func isCenteredTimeSeparator(_ line: OCRLine) -> Bool {
        // Timestamp-shaped text in an actual left/right bubble must remain a message.
        guard (0.55...0.70).contains(line.bounds.midX), line.bounds.height <= 0.03,
              WeChatMessageParser.direction(x: line.bounds.minX, width: line.bounds.width) == .unknown
        else { return false }
        return line.text.trimmingCharacters(in: .whitespacesAndNewlines).range(
            of: #"^(今天|昨天|前天|星期[一二三四五六日天]|\d{1,2}月\d{1,2}日|\d{4}年\d{1,2}月\d{1,2}日)\s*\d{1,2}:\d{2}$"#,
            options: .regularExpression
        ) != nil
    }

    private static func parsedMessages(_ snapshot: OCRSnapshot) -> [ParsedChatMessage] {
        // The old 0.765 cutoff discarded the bottom chat bubble. Share the composer boundary
        // instead; reject boxes crossing it so draft text can never become receipt evidence.
        let chatOnly = OCRSnapshot(
            lines: snapshot.lines.filter {
                $0.bounds.maxY < WeChatDraftPolicy.draftRegion.minY && !isCenteredTimeSeparator($0)
            },
            statistics: snapshot.statistics
        )
        return WeChatMessageParser.parse(chatOnly, maxMessages: 64).messages
    }

    public static func readMessages(_ snapshot: OCRSnapshot) -> [NativeReadMessage] {
        parsedMessages(snapshot).map {
            NativeReadMessage(text: $0.text, direction: $0.direction.rawValue,
                complete: snapshot.statistics.completeRecognition && $0.direction != .unknown)
        }
    }

    public static func receiptSignatures(_ snapshot: OCRSnapshot) -> [SendMessageSignature] {
        parsedMessages(snapshot).map {
            SendMessageSignature(digest: digest($0.text), direction: $0.direction.rawValue,
                                 complete: snapshot.statistics.completeRecognition && $0.direction != .unknown,
                                 continuityDigest: digest(continuityText($0.text)))
        }
    }

    public static func observation(_ raw: WeChatComposerObservation, frontmost: Bool) -> SendObservation {
        let frame = raw.window.frame
        let layout = [frame.minX, frame.minY, frame.width, frame.height, raw.window.scale]
            .map { String(format: "%.4f", Double($0)) }.joined(separator: ":")
        return SendObservation(
            applicationSession: raw.privateSnapshot.applicationSessionFingerprint,
            conversation: raw.privateSnapshot.conversationFingerprint,
            windowRef: String(raw.window.id),
            layoutRef: digest(layout),
            frontmost: frontmost,
            conversationResolved: !raw.privateSnapshot.partialReasons.contains("CONVERSATION_IDENTITY_UNRESOLVED"),
            draftState: WeChatDraftPolicy.readState(raw.draftSnapshot),
            draftText: WeChatDraftPolicy.observedText(raw.draftSnapshot),
            messages: receiptSignatures(raw.chatSnapshot),
            sendButton: sendButton(raw.controlSnapshot),
            evidenceRevision: evidenceRevision
        )
    }

    /// A new matching ME message after an unambiguous suffix/prefix overlap, not a delivery ACK.
    public static func verifiedOutgoing(before: SendObservation, after: SendObservation, text: String) -> Bool {
        guard before.evidenceRevision == evidenceRevision, after.evidenceRevision == evidenceRevision,
              before.sameSurface(as: after), after.frontmost,
              after.draftState == .emptyHeuristic,
              before.messages.allSatisfy(\.complete), after.messages.allSatisfy(\.complete)
        else { return false }
        let expected = digest(text)
        if before.messages.isEmpty {
            return after.messages.count == 1 && after.messages[0].direction == "ME"
                && after.messages[0].digest == expected
        }
        let upper = min(before.messages.count, after.messages.count)
        let lower = min(2, before.messages.count)
        guard upper >= lower else { return false }
        let overlaps = (lower...upper).filter { count in
            zip(before.messages.suffix(count), after.messages.prefix(count))
                .allSatisfy { $0.sameContext(as: $1) }
        }
        guard overlaps.count == 1, let overlap = overlaps.first else { return false }
        let added = after.messages.dropFirst(overlap)
        return added.filter { $0.direction == "ME" && $0.digest == expected }.count == 1
    }
}
