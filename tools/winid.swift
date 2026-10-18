import CoreGraphics
import Foundation

// Prints the window id of the first on-screen window owned by a process with the given name.
let owner = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "rsdisk"
let list = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
for w in list where (w[kCGWindowOwnerName as String] as? String) == owner {
    if let id = w[kCGWindowNumber as String] as? Int, (w[kCGWindowLayer as String] as? Int) == 0 {
        print(id)
        exit(0)
    }
}
exit(1)
