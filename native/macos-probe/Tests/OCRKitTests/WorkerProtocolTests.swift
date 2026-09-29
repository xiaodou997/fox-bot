import XCTest
@testable import OCRKit

final class WorkerProtocolTests: XCTestCase {
    func testCommandRoundTripAndClosedActionNames() throws {
        let command = OCRWorkerCommand(id: "req_01", command: .captureOCR,
                                       app: .wechat, focusedWindow: true)
        let data = try JSONEncoder().encode(command)
        let decoded = try JSONDecoder().decode(OCRWorkerCommand.self, from: data)
        XCTAssertEqual(decoded.id, "req_01")
        XCTAssertEqual(decoded.command, .captureOCR)
        XCTAssertEqual(decoded.app, .wechat)
        XCTAssertEqual(decoded.focusedWindow, true)
        XCTAssertEqual(OCRWorkerAction.captureSnapshot.rawValue, "capture_snapshot")
    }

    func testSnakeCaseWorkerWireFormatDecodesFocusedWindow() throws {
        let data = Data(#"{"id":"req","command":"capture_ocr","app":"wechat","focused_window":true}"#.utf8)
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let decoded = try decoder.decode(OCRWorkerCommand.self, from: data)
        XCTAssertEqual(decoded.command, .captureOCR)
        XCTAssertEqual(decoded.app, .wechat)
        XCTAssertEqual(decoded.focusedWindow, true)
    }

    func testRequestIdIsBoundedAndDoesNotAcceptArbitraryText() {
        XCTAssertTrue(OCRWorkerCommand(id: "abc-123_DEF", command: .warmup).validID)
        XCTAssertFalse(OCRWorkerCommand(id: "", command: .warmup).validID)
        XCTAssertFalse(OCRWorkerCommand(id: "contains space", command: .warmup).validID)
        XCTAssertFalse(OCRWorkerCommand(id: String(repeating: "a", count: 65), command: .warmup).validID)
    }

    func testWarmupReplyContainsNoRawTextFields() throws {
        let warmup = OCRWarmupSummary(elapsedMilliseconds: 123, lineCount: 0, succeeded: true)
        let reply = OCRWorkerReply(id: "warm", status: "WARMED", warmup: warmup)
        let json = String(decoding: try JSONEncoder().encode(reply), as: UTF8.self)
        XCTAssertFalse(json.contains("text"))
        XCTAssertFalse(json.contains("image"))
        XCTAssertFalse(json.contains("bounds"))
    }

    func testPrivateSnapshotWireCarriesOnlyHashedIdentityAndMessages() throws {
        let snapshot = PrivateMessageSnapshot(
            schemaVersion: "foxbot.private-message-snapshot.v1",
            strategy: WeChatMessageParser.strategy,
            applicationSessionFingerprint: String(repeating: "c", count: 64),
            conversationFingerprint: String(repeating: "a", count: 64),
            partialReasons: ["HEURISTIC_REGION"],
            messages: [PrivateBridgeMessage(text: "synthetic", direction: .them,
                                            senderFingerprint: String(repeating: "b", count: 64),
                                            complete: true)])
        let reply = OCRWorkerReply(id: "private", status: "SNAPSHOT", privateSnapshot: snapshot)
        let json = String(decoding: try JSONEncoder().encode(reply), as: UTF8.self)
        XCTAssertTrue(json.contains("synthetic"))
        XCTAssertFalse(json.contains("sender_name"))
        XCTAssertFalse(json.contains("window_id"))
    }
}
