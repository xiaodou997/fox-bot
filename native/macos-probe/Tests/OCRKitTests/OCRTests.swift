import XCTest
import CoreGraphics
import CoreText
import Foundation
@testable import OCRKit

func imageFixture(dark: Bool = false, text: Bool = true, width: Int = 1200, height: Int = 480) -> CGImage {
    let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8,
                            bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
    context.setFillColor(CGColor(gray: dark ? 0 : 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    if text {
        let font = CTFontCreateWithName("PingFangSC-Regular" as CFString, 48, nil)
        for (index, content) in ["FOX123 ORDER789", "订单编号 123456", "Price 198.50"].enumerated() {
            let attributes: [NSAttributedString.Key: Any] = [
                NSAttributedString.Key(kCTFontAttributeName as String): font,
                NSAttributedString.Key(kCTForegroundColorAttributeName as String): CGColor(gray: dark ? 1 : 0, alpha: 1)
            ]
            let line = CTLineCreateWithAttributedString(NSAttributedString(string: content, attributes: attributes))
            context.textPosition = CGPoint(x: 48, y: height - 100 - index * 120)
            CTLineDraw(line, context)
        }
    }
    return context.makeImage()!
}

final class OCRTests: XCTestCase {
    func testActualLocalVisionReadsLightSyntheticChineseLatinAndNumbers() throws {
        let result = try VisionOCR.recognize(imageFixture())
        let compact = result.lines.map(\.text).joined().filter { !$0.isWhitespace }
        for token in ["FOX123", "ORDER789", "订单编号", "123456", "198.50"] { XCTAssertTrue(compact.contains(token)) }
        XCTAssertTrue(result.statistics.completeRecognition)
        XCTAssertFalse(result.statistics.languageCorrection)
        XCTAssertEqual(result.statistics.requestRevision, 3)
        XCTAssertEqual(result.lines.map { $0.bounds.minY }, result.lines.map { $0.bounds.minY }.sorted())
    }
    func testActualLocalVisionReadsDarkSyntheticFixture() throws {
        let result = try VisionOCR.recognize(imageFixture(dark: true))
        let compact = result.lines.map(\.text).joined().filter { !$0.isWhitespace }
        XCTAssertTrue(compact.contains("123456")); XCTAssertTrue(compact.contains("198.50"))
        XCTAssertTrue(compact.contains("订单编号"))
    }
    func testActualBlankImageIsEmptyNotAnAssumedEmptyConversation() throws {
        let result = try VisionOCR.recognize(imageFixture(text: false))
        XCTAssertEqual(result.statistics.lineCount, 0)
        XCTAssertTrue(result.lines.isEmpty)
    }
    func testRetinaSizingIsBoundedWithoutUnrequestedUpscaling() throws {
        let plan = try ImagePlan.make(size: CGSize(width: 1000, height: 600), scale: 2)
        XCTAssertEqual(plan.width, 2000); XCTAssertEqual(plan.height, 1200); XCTAssertFalse(plan.downscaled)
    }
    func testLargeCaptureHasExplicitDownscalingAndPixelBudget() throws {
        let plan = try ImagePlan.make(size: CGSize(width: 6000, height: 4000), scale: 2)
        XCTAssertTrue(plan.downscaled)
        XCTAssertLessThanOrEqual(plan.width * plan.height, ImagePlan.maxPixels)
        XCTAssertLessThanOrEqual(max(plan.width, plan.height), ImagePlan.maxDimension)
    }
    func testInvalidSizesAndScalesFailBeforeCapture() {
        for size in [CGSize.zero, CGSize(width: -1, height: 100), CGSize(width: CGFloat.infinity, height: 100), CGSize(width: 1000000, height: 100)] {
            XCTAssertThrowsError(try ImagePlan.make(size: size, scale: 2))
        }
        for scale in [Double.nan, .infinity, 0, -1, 100] {
            XCTAssertThrowsError(try ImagePlan.make(size: CGSize(width: 100, height: 100), scale: scale))
        }
        XCTAssertFalse(ImagePlan.accepts(width: Int.max, height: Int.max))
    }
    func testBottomLeftConversionIsImageRelative() throws {
        let box = try topLeftBounds(CGRect(x: 0.1, y: 0.7, width: 0.2, height: 0.1))
        XCTAssertEqual(box.minY, 0.2, accuracy: 0.00001); XCTAssertEqual(box.minX, 0.1)
        XCTAssertThrowsError(try topLeftBounds(CGRect(x: -0.1, y: 0, width: 1, height: 1)))
        XCTAssertThrowsError(try topLeftBounds(CGRect(x: 0, y: 0, width: 2, height: 1)))
    }
    func testInvalidObservationAndLowConfidenceAreExplicit() {
        let valid = OCRLine(text: "synthetic", confidence: 0.2, bounds: CGRect(x: 0, y: 0, width: 0.4, height: 0.1))
        let invalid = OCRLine(text: "synthetic", confidence: .nan, bounds: .zero)
        let result = VisionOCR.bounded([invalid, valid])
        XCTAssertEqual(result.statistics.lineCount, 1); XCTAssertEqual(result.statistics.lowConfidenceLines, 1)
        XCTAssertEqual(result.statistics.partialReasons, ["INVALID_OBSERVATION"])
    }
    func testTextLimitDoesNotSilentlyInventOrRepairText() {
        let line = OCRLine(text: String(repeating: "x", count: 5000), confidence: 1,
                           bounds: CGRect(x: 0, y: 0, width: 1, height: 0.1))
        let result = VisionOCR.bounded([line])
        XCTAssertTrue(result.lines.isEmpty); XCTAssertEqual(result.statistics.partialReasons, ["TEXT_LIMIT"])
    }
    func testLineLimitIsPartialAndBoundsRetained() {
        let line = OCRLine(text: "line", confidence: 1, bounds: CGRect(x: 0, y: 0, width: 1, height: 0.1))
        let result = VisionOCR.bounded(Array(repeating: line, count: 600))
        XCTAssertEqual(result.lines.count, 512); XCTAssertEqual(result.statistics.partialReasons, ["LINE_LIMIT"])
        XCTAssertFalse(result.statistics.completeRecognition)
    }
    func testMissingCandidatesArePartialNotAnEmptySuccessfulRead() {
        let result = VisionOCR.bounded([], missingCandidates: true)
        XCTAssertFalse(result.statistics.completeRecognition)
        XCTAssertEqual(result.statistics.partialReasons, ["INVALID_OBSERVATION"])
        XCTAssertTrue(result.lines.isEmpty)
    }
    func testStatisticsDoNotSerializeRecognizedTextOrBoxes() throws {
        let result = VisionOCR.bounded([OCRLine(text: "SYNTHETIC_PRIVATE_987", confidence: 1,
                                               bounds: CGRect(x: 0, y: 0, width: 1, height: 0.1))])
        let data = String(decoding: try JSONEncoder().encode(result.statistics), as: UTF8.self)
        XCTAssertFalse(data.contains("SYNTHETIC_PRIVATE")); XCTAssertFalse(data.contains("bounds"))
    }
}
