import CoreGraphics
import XCTest
@testable import OCRKit

final class SendPolicyTests: XCTestCase {
    private func message(_ text: String, me: Bool = false, complete: Bool = true) -> SendMessageSignature {
        SendMessageSignature(digest: WeChatSendPolicy.digest(text), direction: me ? "ME" : "THEM", complete: complete)
    }
    private func snapshot(_ messages: [SendMessageSignature], draft: DraftReadState = .emptyHeuristic,
                          conversation: String = "chat") -> SendObservation {
        SendObservation(applicationSession: "app", conversation: conversation, windowRef: "w", layoutRef: "l",
                        frontmost: true, conversationResolved: true, draftState: draft, draftText: "",
                        messages: messages, sendButton: nil, evidenceRevision: WeChatSendPolicy.evidenceRevision)
    }
    func testNewMatchingOutgoingIsObservedNotDelivered() {
        let before = snapshot([message("a"), message("b")])
        let after = snapshot(before.messages + [message("reply", me: true)])
        XCTAssertTrue(WeChatSendPolicy.verifiedOutgoing(before: before, after: after, text: "reply"))
    }
    func testOldMatchingMessageIsNotAReceipt() {
        let before = snapshot([message("a"), message("reply", me: true)])
        XCTAssertFalse(WeChatSendPolicy.verifiedOutgoing(before: before, after: before, text: "reply"))
    }
    func testRepeatedTextCanBeANewMessageWithContinuity() {
        let before = snapshot([message("reply", me: true), message("question")])
        let after = snapshot(before.messages + [message("reply", me: true)])
        XCTAssertTrue(WeChatSendPolicy.verifiedOutgoing(before: before, after: after, text: "reply"))
    }
    func testScrolledSuffixWithTwoAnchorsIsAccepted() {
        let before = snapshot([message("a"), message("b"), message("c")])
        let after = snapshot([message("b"), message("c"), message("reply", me: true)])
        XCTAssertTrue(WeChatSendPolicy.verifiedOutgoing(before: before, after: after, text: "reply"))
    }
    func testAbsentOverlapOrWrongDirectionNeverVerifies() {
        let before = snapshot([message("a"), message("b")])
        for after in [snapshot([message("reply", me: true)]), snapshot(before.messages + [message("reply")])] {
            XCTAssertFalse(WeChatSendPolicy.verifiedOutgoing(before: before, after: after, text: "reply"))
        }
    }
    func testChangedTargetNonemptyDraftAndPartialOCRNeverVerify() {
        let before = snapshot([message("a"), message("b")])
        for after in [snapshot(before.messages + [message("reply", me: true)], conversation: "other"),
                      snapshot(before.messages + [message("reply", me: true)], draft: .nonempty),
                      snapshot(before.messages + [message("reply", me: true, complete: false)])] {
            XCTAssertFalse(WeChatSendPolicy.verifiedOutgoing(before: before, after: after, text: "reply"))
        }
    }
    func testAmbiguousRepeatedAnchorsAndDuplicateOutputDoNotVerify() {
        let before = snapshot([message("x"), message("x"), message("x")])
        let after = snapshot(before.messages + [message("reply", me: true)])
        XCTAssertFalse(WeChatSendPolicy.verifiedOutgoing(before: before, after: after, text: "reply"))
        let unique = snapshot([message("a"), message("b")])
        XCTAssertFalse(WeChatSendPolicy.verifiedOutgoing(before: unique,
            after: snapshot(unique.messages + [message("reply", me: true), message("reply", me: true)]), text: "reply"))
    }
    func testEmptyHistoryOnlyAcceptsOneNewOwnMessage() {
        XCTAssertTrue(WeChatSendPolicy.verifiedOutgoing(before: snapshot([]),
            after: snapshot([message("reply", me: true)]), text: "reply"))
        XCTAssertFalse(WeChatSendPolicy.verifiedOutgoing(before: snapshot([]),
            after: snapshot([message("unrelated"), message("reply", me: true)]), text: "reply"))
    }
    private func ocr(_ lines: [OCRLine], complete: Bool = true) -> OCRSnapshot {
        OCRSnapshot(lines: lines, statistics: OCRStatistics(lineCount: lines.count,
            characterCount: lines.reduce(0) { $0 + $1.text.count }, lowConfidenceLines: 0,
            partialReasons: complete ? [] : ["OCR_PARTIAL"], completeRecognition: complete))
    }
    func testBottomBubbleAboveComposerBoundaryIsIncluded() {
        let bottom = OCRLine(text: "synthetic reply", confidence: 0.99,
            bounds: CGRect(x: 0.80, y: 0.767, width: 0.15, height: 0.012))
        XCTAssertEqual(WeChatSendPolicy.receiptSignatures(ocr([bottom])), [message("synthetic reply", me: true)])
    }
    func testComposerAndBoundaryCrossingTextAreNeverReceiptEvidence() {
        let draft = OCRLine(text: "synthetic reply", confidence: 0.99,
            bounds: CGRect(x: 0.80, y: 0.82, width: 0.15, height: 0.012))
        let crossing = OCRLine(text: "synthetic reply", confidence: 0.99,
            bounds: CGRect(x: 0.80, y: 0.785, width: 0.15, height: 0.012))
        XCTAssertEqual(WeChatSendPolicy.receiptSignatures(ocr([draft, crossing])), [])
    }
    func testCenteredRelativeTimeSeparatorsDoNotBecomeUnknownMessages() {
        for timestamp in ["今天 18:23", "昨天 09:15", "前天 08:10", "星期一 12:34", "10月1日 12:34"] {
            let line = OCRLine(text: timestamp, confidence: 0.99,
                bounds: CGRect(x: 0.598, y: 0.40, width: 0.07, height: 0.013))
            XCTAssertEqual(WeChatSendPolicy.receiptSignatures(ocr([line])), [], timestamp)
        }
    }
    func testTimestampTextInsideBubblesIsNotDeleted() {
        for x in [0.33, 0.80] {
            let line = OCRLine(text: "今天 18:23", confidence: 0.99,
                bounds: CGRect(x: x, y: 0.40, width: 0.13, height: 0.013))
            XCTAssertEqual(WeChatSendPolicy.receiptSignatures(ocr([line])), [message("今天 18:23", me: x > 0.7)])
        }
    }
    func testIncompleteOCRStillCannotBecomeCompleteReceiptEvidence() {
        let line = OCRLine(text: "synthetic reply", confidence: 0.99,
            bounds: CGRect(x: 0.80, y: 0.767, width: 0.15, height: 0.012))
        XCTAssertEqual(WeChatSendPolicy.receiptSignatures(ocr([line], complete: false)),
                       [message("synthetic reply", me: true, complete: false)])
    }
    func testLegacyAndUnknownEvidenceRevisionsCannotVerifyNewObservations() {
        let current = snapshot([message("a"), message("b")])
        let after = snapshot(current.messages + [message("reply", me: true)])
        let revisions: [String?] = [nil, "FUTURE_REVISION"]
        for revision in revisions {
            var before = current
            before.evidenceRevision = revision
            XCTAssertFalse(WeChatSendPolicy.verifiedOutgoing(before: before, after: after, text: "reply"))
        }
    }

    func testTextBoundsRejectControlKeysBeforeWriting() {
        for text in ["", " hello", "hello\nworld", "\t", "hello\0", String(repeating: "a", count: 81), "x|"] {
            XCTAssertFalse(WeChatSendPolicy.supportedText(text))
        }
        XCTAssertTrue(WeChatSendPolicy.supportedText("FoxBot G3c1 742961"))
        XCTAssertTrue(WeChatSendPolicy.supportedText("测试回复 742961"))
    }
    func testSendButtonRequiresUniqueLabelWithinControlArea() {
        func ocr(_ lines: [OCRLine]) -> OCRSnapshot {
            OCRSnapshot(lines: lines, statistics: OCRStatistics(lineCount: lines.count, characterCount: 2,
                lowConfidenceLines: 0, partialReasons: [], completeRecognition: true))
        }
        let good = OCRLine(text: "发送", confidence: 0.99, bounds: CGRect(x: 0.92, y: 0.93, width: 0.035, height: 0.03))
        let body = OCRLine(text: "发送", confidence: 0.99, bounds: CGRect(x: 0.5, y: 0.5, width: 0.035, height: 0.03))
        XCTAssertNotNil(WeChatSendPolicy.sendButton(ocr([good, body])))
        XCTAssertNil(WeChatSendPolicy.sendButton(ocr([body])))
        XCTAssertNil(WeChatSendPolicy.sendButton(ocr([good, good])))
    }
}
