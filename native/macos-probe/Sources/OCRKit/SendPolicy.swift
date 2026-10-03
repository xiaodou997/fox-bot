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
    public let contentDigest: String?

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

    /// A larger composer can hide older rows at the top without changing the chat.
    /// Only that leading-drop shape is accepted; a new bottom message never is.
    public func hasNoNewMessages(since before: SendObservation) -> Bool {
        if sameMessages(as: before) { return true }
        guard messages.count >= 2, messages.count < before.messages.count else { return false }
        let anchors = before.messages.suffix(messages.count)
        guard zip(anchors, messages).allSatisfy({ $0.sameContext(as: $1) }),
              let first = anchors.first,
              anchors.dropFirst().contains(where: { !first.sameContext(as: $0) })
        else { return false }
        return true
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
    public static let legacyEvidenceRevision = "WECHAT_RECEIPT_V3"
    public static let evidenceRevision = "WECHAT_RECEIPT_V4"
    /// Tight bottom-right crop keeps long composer text from suppressing the button label.
    public static let controlRegion = CGRect(x: 0.90, y: 0.91, width: 0.10, height: 0.09)
    public static let shortUnicodeLimit = 80
    public static let maxTextUTF16 = 512
    public static let maxTextLines = 12

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

    /// Receipt-only content projection. Exact draft verification happens before send;
    /// this removes visual-wrap whitespace without changing any non-whitespace scalar.
    public static func contentText(_ text: String) -> String {
        String(text.unicodeScalars.filter {
            !CharacterSet.whitespacesAndNewlines.contains($0)
        })
    }

    /// Bounded plain text only. LF is allowed; tabs, CR and all other control characters
    /// remain rejected before any native write.
    public static func supportedText(_ text: String) -> Bool {
        !text.isEmpty && text.utf16.count <= maxTextUTF16
            && text.split(separator: "\n", omittingEmptySubsequences: false).count <= maxTextLines
            && text == text.trimmingCharacters(in: .whitespacesAndNewlines)
            && !text.contains("\r")
            && !text.unicodeScalars.contains {
                CharacterSet.controlCharacters.contains($0) && $0.value != 10
            }
            && !text.hasSuffix("|")
    }

    public static func usesPasteboardInput(_ text: String) -> Bool {
        text.contains("\n") || text.utf16.count > shortUnicodeLimit
    }

    public static func sendButton(_ snapshot: OCRSnapshot?) -> SendPoint? {
        guard let snapshot, snapshot.statistics.completeRecognition else { return nil }
        let candidates = snapshot.lines.filter {
            $0.confidence >= 0.65
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
                                 continuityDigest: digest(continuityText($0.text)),
                                 contentDigest: digest(contentText($0.text)))
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

    private static func boundedEditDistance(
        _ left: [Character],
        _ right: [Character],
        limit: Int
    ) -> Int? {
        guard abs(left.count - right.count) <= limit else { return nil }
        var previous = Array(0...right.count)
        for (row, lhs) in left.enumerated() {
            var current = Array(repeating: 0, count: right.count + 1)
            current[0] = row + 1
            var minimum = current[0]
            for (column, rhs) in right.enumerated() {
                let substitution = previous[column] + (lhs == rhs ? 0 : 1)
                current[column + 1] = min(
                    previous[column + 1] + 1,
                    current[column] + 1,
                    substitution
                )
                minimum = min(minimum, current[column + 1])
            }
            guard minimum <= limit else { return nil }
            previous = current
        }
        return previous[right.count] <= limit ? previous[right.count] : nil
    }

    /// Visual OCR is permitted at most two non-whitespace errors for long replies, after the
    /// exact composer copy-readback has already succeeded. The ends must remain anchored.
    public static func receiptTextMatches(expected: String, observed: String) -> Bool {
        let expectedContent = Array(contentText(expected))
        let observedContent = Array(contentText(observed))
        if expectedContent == observedContent { return true }
        guard usesPasteboardInput(expected), expectedContent.count >= 64,
              abs(expectedContent.count - observedContent.count) <= 2,
              expectedContent.prefix(8).elementsEqual(observedContent.prefix(8)),
              expectedContent.suffix(8).elementsEqual(observedContent.suffix(8))
        else { return false }
        return boundedEditDistance(expectedContent, observedContent, limit: 2) != nil
    }

    private static func outgoingEnd(before: SendObservation, after: SendObservation) -> Int? {
        let supportedRevisions = [legacyEvidenceRevision, evidenceRevision]
        guard before.evidenceRevision.map(supportedRevisions.contains) == true,
              after.evidenceRevision.map(supportedRevisions.contains) == true,
              before.sameSurface(as: after), after.frontmost,
              after.draftState == .emptyHeuristic,
              before.messages.allSatisfy(\.complete), after.messages.allSatisfy(\.complete)
        else { return nil }
        if before.messages.isEmpty {
            return after.messages.count == 1 ? 0 : nil
        }

        var candidates: [(start: Int, count: Int)] = []
        for start in after.messages.indices {
            let upper = min(before.messages.count, after.messages.count - start)
            guard upper >= 2 else { continue }
            for count in stride(from: upper, through: 2, by: -1) {
                let matched = zip(before.messages.suffix(count),
                                  after.messages[start..<(start + count)])
                    .allSatisfy { $0.sameContext(as: $1) }
                if matched {
                    candidates.append((start, count))
                    break
                }
            }
        }
        let valid = candidates.filter { after.messages.count - ($0.start + $0.count) == 1 }
        let ends = Set(valid.map { $0.start + $0.count })
        guard ends.count == 1, let end = ends.first else { return nil }
        let aligned = valid.filter { $0.start + $0.count == end }
        if aligned.count > 1, let longest = aligned.max(by: { $0.count < $1.count }) {
            let anchors = before.messages.suffix(longest.count)
            guard let first = anchors.first,
                  anchors.dropFirst().contains(where: { !first.sameContext(as: $0) })
            else { return nil }
        }
        return end
    }

    /// A new matching ME message after an unambiguous context overlap, not a delivery ACK.
    public static func verifiedOutgoing(before: SendObservation, after: SendObservation, text: String) -> Bool {
        guard let end = outgoingEnd(before: before, after: after) else { return false }
        let expected = digest(text)
        let expectedContent = digest(contentText(text))
        let allowContentProjection = usesPasteboardInput(text)
        let added = after.messages.dropFirst(end)
        guard added.count == 1, let message = added.first, message.direction == "ME" else {
            return false
        }
        return message.digest == expected
            || (allowContentProjection && message.contentDigest == expectedContent)
    }

    private static func fuzzyOutgoingEnd(before: SendObservation, after: SendObservation) -> Int? {
        let supportedRevisions = [legacyEvidenceRevision, evidenceRevision]
        guard before.evidenceRevision.map(supportedRevisions.contains) == true,
              after.evidenceRevision.map(supportedRevisions.contains) == true,
              before.sameSurface(as: after), after.frontmost,
              after.draftState == .emptyHeuristic,
              before.messages.allSatisfy(\.complete), after.messages.allSatisfy(\.complete)
        else { return nil }
        if before.messages.isEmpty {
            return (1...3).contains(after.messages.count) ? 0 : nil
        }

        var candidates: [(end: Int, count: Int)] = []
        for start in after.messages.indices {
            let upper = min(before.messages.count, after.messages.count - start)
            guard upper >= 2 else { continue }
            for count in stride(from: upper, through: 2, by: -1) {
                let trailing = after.messages.count - (start + count)
                guard (1...3).contains(trailing) else { continue }
                let left = Array(before.messages.suffix(count))
                let right = Array(after.messages[start..<(start + count)])
                var mismatch = -1
                var valid = true
                for index in left.indices where !left[index].sameContext(as: right[index]) {
                    if mismatch >= 0 {
                        valid = false
                        break
                    }
                    mismatch = index
                }
                if mismatch >= 0 {
                    let index = mismatch
                    valid = valid && count >= 5 && index == count - 1
                        && left[index].direction == "THEM"
                        && right[index].direction == "THEM"
                        && left[index].complete && right[index].complete
                }
                if valid {
                    candidates.append((start + count, count))
                    break
                }
            }
        }
        let ends = Set(candidates.map(\.end))
        guard ends.count == 1, let end = ends.first else { return nil }
        let aligned = candidates.filter { $0.end == end }
        if aligned.count > 1, let longest = aligned.max(by: { $0.count < $1.count }) {
            let anchors = before.messages.suffix(longest.count)
            guard let first = anchors.first,
                  anchors.dropFirst().contains(where: { !first.sameContext(as: $0) })
            else { return nil }
        }
        return end
    }

    /// Private raw text is used only inside the native worker to tolerate bounded OCR errors.
    public static func verifiedOutgoing(
        before: SendObservation,
        after: SendObservation,
        rawAfter: OCRSnapshot,
        text: String
    ) -> Bool {
        if verifiedOutgoing(before: before, after: after, text: text) { return true }
        guard usesPasteboardInput(text),
              let end = fuzzyOutgoingEnd(before: before, after: after),
              (1...3).contains(after.messages.count - end),
              rawAfter.statistics.completeRecognition
        else { return false }
        let parsed = parsedMessages(rawAfter)
        guard parsed.count == after.messages.count,
              parsed[end].direction == .me,
              parsed[end...].allSatisfy({ $0.direction != .unknown })
        else { return false }
        let observed = parsed[end...].map(\.text).joined(separator: "\n")
        return receiptTextMatches(expected: text, observed: observed)
    }
}
