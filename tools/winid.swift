import CoreGraphics
import Foundation

// Prints the id of the largest on-screen window owned by a process with the given name, for
// `screencapture -l`. Any layer counts: screenshot launches float above normal windows.
let owner = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "rsdisk"
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
let area = { (w: [String: Any]) -> Double in
    let b = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
    return (b["Width"] ?? 0) * (b["Height"] ?? 0)
}
let mine = list.filter { ($0[kCGWindowOwnerName as String] as? String) == owner }
if let best = mine.max(by: { area($0) < area($1) }), let id = best[kCGWindowNumber as String] as? Int {
    print(id)
    exit(0)
}
exit(1)
