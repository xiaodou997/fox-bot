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
}
