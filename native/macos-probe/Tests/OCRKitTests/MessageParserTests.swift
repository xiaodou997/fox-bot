import XCTest
import CoreGraphics
@testable import OCRKit

private func parserSnapshot(_ lines: [OCRLine], complete: Bool = true) -> OCRSnapshot {
    OCRSnapshot(lines: lines, statistics: OCRStatistics(
        lineCount: lines.count,
        characterCount: lines.reduce(0) { $0 + $1.text.count },
        lowConfidenceLines: lines.filter { $0.confidence < 0.7 }.count,
        partialReasons: complete ? [] : ["INVALID_OBSERVATION"],
        completeRecognition: complete
    ))
}

private func parserLine(_ text: String, _ x: CGFloat, _ y: CGFloat,
                        _ width: CGFloat = 0.20, _ height: CGFloat = 0.03,
                        confidence: Float = 0.95) -> OCRLine {
    OCRLine(text: text, confidence: confidence,
            bounds: CGRect(x: x, y: y, width: width, height: height))
}

final class MessageParserTests: XCTestCase {
    func testDirectionIsConservativeForWideAndCentralText() {
        XCTAssertEqual(WeChatMessageParser.direction(x: 0.70, width: 0.15), .me)
        XCTAssertEqual(WeChatMessageParser.direction(x: 0.38, width: 0.20), .them)
        XCTAssertEqual(WeChatMessageParser.direction(x: 0.53, width: 0.15), .unknown)
        XCTAssertEqual(WeChatMessageParser.direction(x: 0.52, width: 0.31), .me)
    }

    func testReadRegionIncludesTopTitleWhileChatRegionExcludesHeader() {
        XCTAssertEqual(WeChatMessageParser.readRegion.minY, 0.0)
        XCTAssertLessThanOrEqual(WeChatMessageParser.readRegion.minX, 0.275)
        XCTAssertEqual(WeChatMessageParser.chatRegion.minY, 0.10)
    }

    func testRegionAndNoiseAreExcludedWithoutInventingMessages() {
        let snapshot = parserSnapshot([
            parserLine("会话列表", 0.05, 0.30),
            parserLine("搜索", 0.40, 0.20),
            parserLine("12:30", 0.40, 0.30),
            parserLine("有效消息", 0.40, 0.40)
        ])
        let result = WeChatMessageParser.parse(snapshot)
        XCTAssertEqual(result.messages.count, 1)
        XCTAssertEqual(result.messages[0].text, "有效消息")
        XCTAssertEqual(result.messages[0].direction, .them)
        XCTAssertTrue(result.partialReasons.contains("HEURISTIC_REGION"))
    }

    func testLowerVisibleChatIsIncludedButInputAreaRemainsExcluded() {
        let snapshot = parserSnapshot([
            parserLine("群成员", 0.33, 0.73, 0.06, 0.018),
            parserLine("底部可见正文", 0.335, 0.79, 0.14, 0.025),
            parserLine("输入框草稿", 0.40, 0.88, 0.16, 0.03)
        ])
        let result = WeChatMessageParser.parse(snapshot)
        XCTAssertEqual(result.messages.count, 1)
        XCTAssertEqual(result.messages[0].text, "底部可见正文")
        XCTAssertEqual(result.messages[0].sender, "群成员")
    }

    func testDateSeparatorsAndCallSystemEventsAreExcludedConservatively() {
        let snapshot = parserSnapshot([
            parserLine("2025年12月6日 20:17", 0.54, 0.20),
            parserLine("星期一 11:49", 0.54, 0.24),
            parserLine("通话时长00:06 9", 0.70, 0.28),
            parserLine("对方已拒绝风", 0.70, 0.32),
            parserLine("已取消の", 0.70, 0.36),
            parserLine("• 已在其它设备拒绝", 0.40, 0.40),
            parserLine("我说对方已拒绝我", 0.40, 0.48),
            parserLine("已取消订单", 0.40, 0.54),
            parserLine("通话时长00:06后继续聊", 0.40, 0.60),
        ])
        let result = WeChatMessageParser.parse(snapshot)
        XCTAssertEqual(result.messages.count, 1)
        XCTAssertEqual(result.messages[0].lines, [
            "我说对方已拒绝我",
            "已取消订单",
            "通话时长00:06后继续聊",
        ])
        XCTAssertFalse(result.messages.contains { $0.direction == .unknown })
    }

    func testMultilineMessageFoldsOnlyWhenAlignedAndCompatible() {
        let snapshot = parserSnapshot([
            parserLine("第一行", 0.40, 0.30, 0.20, 0.03),
            parserLine("第二行", 0.405, 0.355, 0.18, 0.03),
            parserLine("我方", 0.72, 0.50, 0.12, 0.03)
        ])
        let result = WeChatMessageParser.parse(snapshot)
        XCTAssertEqual(result.messages.count, 2)
        XCTAssertEqual(result.messages[0].lines, ["第一行", "第二行"])
        XCTAssertEqual(result.messages[0].direction, .them)
        XCTAssertEqual(result.messages[1].direction, .me)
    }

    func testGroupSenderHeaderIsAttachedButNeverSerializedInSummary() throws {
        let snapshot = parserSnapshot([
            parserLine("小明", 0.40, 0.25, 0.08, 0.018),
            parserLine("这是一条群消息", 0.405, 0.295, 0.24, 0.032)
        ])
        let result = WeChatMessageParser.parse(snapshot)
        XCTAssertEqual(result.messages.count, 1)
        XCTAssertEqual(result.messages[0].sender, "小明")
        XCTAssertEqual(result.summary.senderLabeledCount, 1)
        let json = String(decoding: try JSONEncoder().encode(result.summary), as: UTF8.self)
        XCTAssertFalse(json.contains("小明"))
        XCTAssertFalse(json.contains("群消息"))
        XCTAssertFalse(json.contains("bounds"))
    }

    func testUnknownDirectionIsPreservedAndMakesSnapshotPartial() {
        let result = WeChatMessageParser.parse(parserSnapshot([
            parserLine("中间区域", 0.54, 0.35, 0.14, 0.03)
        ]))
        XCTAssertEqual(result.messages.first?.direction, .unknown)
        XCTAssertEqual(result.summary.unknownCount, 1)
        XCTAssertTrue(result.partialReasons.contains("UNKNOWN_DIRECTION"))
        XCTAssertFalse(result.summary.complete)
    }

    func testOcrPartialReasonPropagatesWithoutRepair() {
        let result = WeChatMessageParser.parse(parserSnapshot([
            parserLine("原始识别", 0.40, 0.35)
        ], complete: false))
        XCTAssertEqual(result.messages.first?.text, "原始识别")
        XCTAssertTrue(result.partialReasons.contains("OCR_PARTIAL"))
    }

    func testConversationFingerprintNormalizesMemberCountAndDoesNotExposeTitle() {
        let first = parserSnapshot([
            parserLine("测试群（12）", 0.40, 0.04, 0.15, 0.03),
            parserLine("消息", 0.40, 0.30)
        ])
        let second = parserSnapshot([
            parserLine("测试群 (99)", 0.40, 0.04, 0.15, 0.03),
            parserLine("消息", 0.40, 0.30)
        ])
        let a = WeChatMessageParser.conversationFingerprint(first)
        let b = WeChatMessageParser.conversationFingerprint(second)
        XCTAssertEqual(a, b)
        XCTAssertEqual(a?.count, 64)
        XCTAssertFalse(a?.contains("测试群") ?? true)
    }

    func testConversationFingerprintAcceptsWideLayoutTitleNearRightSide() {
        let snapshot = parserSnapshot([
            parserLine("联系人名称", 0.81, 0.05, 0.11, 0.02),
            parserLine("下一行正文", 0.82, 0.13, 0.10, 0.02)
        ])
        let fingerprint = WeChatMessageParser.conversationFingerprint(snapshot)
        XCTAssertEqual(fingerprint?.count, 64)
        XCTAssertFalse(fingerprint?.contains("联系人") ?? true)
    }

    func testConversationFingerprintAcceptsGroupTitleJustLeftOfChatBody() {
        let snapshot = parserSnapshot([
            parserLine("测试群聊标题", 0.2799, 0.041, 0.23, 0.018),
            parserLine("会话列表文字", 0.20, 0.05, 0.08, 0.02),
            parserLine("正文", 0.40, 0.30, 0.12, 0.03)
        ])
        let fingerprint = WeChatMessageParser.conversationFingerprint(snapshot)
        XCTAssertEqual(fingerprint?.count, 64)
        let parsed = WeChatMessageParser.parse(snapshot)
        XCTAssertEqual(parsed.messages.map(\.text), ["正文"])
    }

    func testPrivateSnapshotRequiresHeaderIdentityAndHashesSender() throws {
        let snapshot = parserSnapshot([
            parserLine("会话名", 0.40, 0.04, 0.12, 0.03),
            parserLine("群成员", 0.40, 0.28, 0.08, 0.018),
            parserLine("正文", 0.405, 0.325, 0.18, 0.032)
        ])
        let result = try XCTUnwrap(WeChatMessageParser.privateSnapshot(
            snapshot, applicationSessionFingerprint: String(repeating: "c", count: 64)))
        XCTAssertEqual(result.messages.count, 1)
        XCTAssertEqual(result.messages[0].direction, .them)
        XCTAssertEqual(result.messages[0].senderFingerprint?.count, 64)
        let data = String(decoding: try JSONEncoder().encode(result), as: UTF8.self)
        XCTAssertTrue(data.contains("正文"))
        XCTAssertFalse(data.contains("群成员"))
        XCTAssertFalse(data.contains("会话名"))

        let noHeader = parserSnapshot([parserLine("正文", 0.40, 0.30)])
        XCTAssertNil(WeChatMessageParser.privateSnapshot(
            noHeader, applicationSessionFingerprint: String(repeating: "c", count: 64)))
    }

    func testApplicationSessionFingerprintIsStableWithinLaunchAndChangesAcrossLaunch() {
        let a = WeChatMessageParser.applicationSessionFingerprint(
            bundleID: "com.tencent.xinWeChat", launchTime: 123.456)
        let b = WeChatMessageParser.applicationSessionFingerprint(
            bundleID: "com.tencent.xinWeChat", launchTime: 123.456)
        let c = WeChatMessageParser.applicationSessionFingerprint(
            bundleID: "com.tencent.xinWeChat", launchTime: 124.456)
        XCTAssertEqual(a, b)
        XCTAssertNotEqual(a, c)
        XCTAssertEqual(a?.count, 64)
        XCTAssertNil(WeChatMessageParser.applicationSessionFingerprint(
            bundleID: "com.tencent.xinWeChat", launchTime: .nan))
    }
}
