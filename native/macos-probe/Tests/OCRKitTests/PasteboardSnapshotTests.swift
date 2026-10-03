import AppKit
import XCTest
@testable import OCRKit

final class PasteboardSnapshotTests: XCTestCase {
    func testSnapshotRestoresTextAndCustomTypes() throws {
        let pasteboard = NSPasteboard.withUniqueName()
        let custom = NSPasteboard.PasteboardType("com.foxbot.synthetic")
        let item = NSPasteboardItem()
        item.setString("original", forType: .string)
        item.setData(Data([1, 2, 3, 4]), forType: custom)
        XCTAssertTrue(pasteboard.writeObjects([item]))
        let snapshot = try XCTUnwrap(PasteboardSnapshot(pasteboard: pasteboard))

        pasteboard.clearContents()
        XCTAssertTrue(pasteboard.setString("temporary", forType: .string))
        XCTAssertTrue(snapshot.restore(to: pasteboard))
        XCTAssertEqual(pasteboard.string(forType: .string), "original")
        XCTAssertEqual(pasteboard.data(forType: custom), Data([1, 2, 3, 4]))
    }

    func testSnapshotRejectsClipboardLargerThanBound() {
        let pasteboard = NSPasteboard.withUniqueName()
        pasteboard.clearContents()
        XCTAssertTrue(pasteboard.setString(String(repeating: "x", count: 64), forType: .string))
        XCTAssertNil(PasteboardSnapshot(pasteboard: pasteboard, maxBytes: 8))
    }
}
