import AppKit
import Foundation

/// A bounded in-memory copy of the general pasteboard. Long or multiline
/// draft verification uses it so a temporary paste/copy round trip does not
/// leave the user's clipboard replaced by FoxBot text.
public struct PasteboardSnapshot {
    private let items: [[NSPasteboard.PasteboardType: Data]]

    public init?(pasteboard: NSPasteboard, maxBytes: Int = 8 * 1024 * 1024) {
        guard maxBytes > 0 else { return nil }
        let sourceItems = pasteboard.pasteboardItems ?? []
        guard sourceItems.count <= 32 else { return nil }

        var totalBytes = 0
        var captured: [[NSPasteboard.PasteboardType: Data]] = []
        captured.reserveCapacity(sourceItems.count)
        for source in sourceItems {
            guard source.types.count <= 64 else { return nil }
            var entry: [NSPasteboard.PasteboardType: Data] = [:]
            for type in source.types {
                guard let data = source.data(forType: type) else { return nil }
                totalBytes += data.count
                guard totalBytes <= maxBytes else { return nil }
                entry[type] = data
            }
            captured.append(entry)
        }
        items = captured
    }

    @discardableResult
    public func restore(to pasteboard: NSPasteboard) -> Bool {
        pasteboard.clearContents()
        guard !items.isEmpty else { return true }

        let restored: [NSPasteboardItem] = items.map { values in
            let item = NSPasteboardItem()
            for (type, data) in values {
                item.setData(data, forType: type)
            }
            return item
        }
        return pasteboard.writeObjects(restored)
    }
}
