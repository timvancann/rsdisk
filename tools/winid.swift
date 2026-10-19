import CoreGraphics
import Foundation

// Prints the id of the largest on-screen window owned by a process with the given name, for
// `screencapture -l`; a second argument picks the window with that title instead. Any layer
// counts: screenshot launches float above normal windows.
let args = CommandLine.arguments
let owner = args.count > 1 ? args[1] : "rsdisk"
let title = args.count > 2 ? args[2] : nil
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
let area = { (w: [String: Any]) -> Double in
    let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
    return (b["Width"] ?? 0) * (b["Height"] ?? 0)
}
let mine = list.filter {
    ($0[kCGWindowOwnerName as String] as? String) == owner
        && (title == nil || ($0[kCGWindowName as String] as? String) == title)
}
if let best = mine.max(by: { area($0) < area($1) }), let id = best[kCGWindowNumber as String] as? Int {
    print(id)
    exit(0)
}
exit(1)
