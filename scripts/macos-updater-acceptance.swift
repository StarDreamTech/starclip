// Disposable GitHub runner only. Exercise the shipped WebKit accessibility
// controls; do not inject JavaScript, invoke IPC, grant permissions or read a clipboard.
import AppKit
import ApplicationServices

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(1)
}
func emit(_ value: Any) {
    let bytes = try! JSONSerialization.data(withJSONObject: value, options: [.sortedKeys])
    print(String(data: bytes, encoding: .utf8)!)
}
let environment = ProcessInfo.processInfo.environment
guard environment["GITHUB_ACTIONS"] == "true",
      environment["RUNNER_ENVIRONMENT"] == "github-hosted",
      environment["GITHUB_REPOSITORY"] == "StarDreamTech/starclip" else {
    fail("Only a disposable GitHub-hosted Starclip runner may use this UI helper")
}
let args = CommandLine.arguments
guard args.count >= 2 else { fail("Expected preflight, processes, inspect, prepare, find, press or dump") }
if args[1] == "preflight" {
    guard AXIsProcessTrusted() else {
        fail("Accessibility is not authorized on this runner; UI acceptance is unverified. No TCC or privacy settings were changed.")
    }
    emit(["accessibilityTrusted": true]); exit(0)
}
guard args.count >= 3 else { fail("Expected an owned bundle path or PID") }
if args[1] == "processes" {
    let path = URL(fileURLWithPath: args[2]).resolvingSymlinksInPath().standardizedFileURL.path
    let pids = NSRunningApplication.runningApplications(withBundleIdentifier: "com.starclip.desktop")
        .filter { $0.bundleURL?.resolvingSymlinksInPath().standardizedFileURL.path == path && !$0.isTerminated }
        .map { $0.processIdentifier }
    emit(["pids": pids]); exit(0)
}
if args[1] == "diagnose-processes" {
    let temporary = URL(fileURLWithPath: environment["RUNNER_TEMP"]!).resolvingSymlinksInPath().path + "/"
    let owned = NSRunningApplication.runningApplications(withBundleIdentifier: "com.starclip.desktop")
        .filter { $0.bundleURL?.resolvingSymlinksInPath().path.hasPrefix(temporary) == true }
    emit(owned.map { ["pid": $0.processIdentifier, "bundle": $0.bundleURL?.path ?? "",
                     "executable": $0.executableURL?.path ?? "", "terminated": $0.isTerminated,
                     "finishedLaunching": $0.isFinishedLaunching, "active": $0.isActive] as [String: Any] })
    exit(0)
}
guard let pid = Int32(args[2]), pid > 0 else { fail("Expected a positive owned PID") }
let runningApp = NSRunningApplication(processIdentifier: pid)
if args[1] == "inspect" && (runningApp == nil || runningApp?.bundleIdentifier == nil) {
    emit(["window": false]); exit(0)
}
guard let app = runningApp,
      app.bundleIdentifier == "com.starclip.desktop",
      let bundle = app.bundleURL?.resolvingSymlinksInPath().standardizedFileURL,
      let temporary = environment["RUNNER_TEMP"],
      bundle.path.hasPrefix(URL(fileURLWithPath: temporary).resolvingSymlinksInPath().standardizedFileURL.path + "/"),
      bundle.path.contains("starclip-macos-updater-"), bundle.path.hasSuffix("/Starclip.app") else {
    fail("Refusing to inspect or operate a process outside the runner-owned application")
}
let windowInfo = CGWindowListCopyWindowInfo([.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID)
    as? [[String: Any]] ?? []
let visible = windowInfo.contains { row in
    guard row[kCGWindowOwnerPID as String] as? Int32 == pid,
          row[kCGWindowLayer as String] as? Int == 0,
          let bounds = row[kCGWindowBounds as String] as? [String: CGFloat] else { return false }
    return (bounds["Width"] ?? 0) >= 860 && (bounds["Height"] ?? 0) >= 620
}
if args[1] == "inspect" {
    emit(["window": visible && app.isFinishedLaunching && !app.isTerminated]); exit(0)
}
guard AXIsProcessTrusted() else { fail("Accessibility authorization is unavailable") }
func attribute(_ element: AXUIElement, _ key: String) -> CFTypeRef? {
    var value: CFTypeRef?
    return AXUIElementCopyAttributeValue(element, key as CFString, &value) == .success ? value : nil
}
func strings(_ node: AXUIElement) -> [String] {
    var result = [kAXTitleAttribute, kAXDescriptionAttribute, kAXValueAttribute]
        .compactMap { attribute(node, $0) as? String }
    // WebKit can expose a button's visible label only as child static text.
    // Limit this to direct children, so parent groups never impersonate buttons.
    let children = attribute(node, kAXChildrenAttribute) as? [AXUIElement] ?? []
    for child in children where attribute(child, kAXRoleAttribute) as? String == kAXStaticTextRole {
        result.append(contentsOf: [kAXTitleAttribute, kAXValueAttribute].compactMap { attribute(child, $0) as? String })
    }
    return result.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
}
func walk(_ root: AXUIElement) -> [AXUIElement] {
    var queue = [root], result: [AXUIElement] = []
    var visited = Set<CFHashCode>()
    while !queue.isEmpty && result.count < 4000 {
        let node = queue.removeLast()
        if !visited.insert(CFHash(node)).inserted { continue }
        result.append(node)
        queue.append(contentsOf: attribute(node, kAXChildrenAttribute) as? [AXUIElement] ?? [])
    }
    return result
}
let application = AXUIElementCreateApplication(pid)
AXUIElementSetMessagingTimeout(application, 3)
if args[1] == "prepare" {
    // The runner already grants AX access. Foreground this owned test app once
    // per launch so its WebKit content is available; no system setting changes.
    let requested = app.activate(options: [])
    emit(["activationRequested": requested, "activated": app.isActive]); exit(0)
}
let roots = attribute(application, kAXWindowsAttribute) as? [AXUIElement] ?? []
var nodes = roots.flatMap(walk)
if args[1] == "dump" {
    emit(nodes.map { ["role": attribute($0, kAXRoleAttribute) as? String ?? "",
                      "names": strings($0).joined(separator: " | ")] }); exit(0)
}
guard args.count >= 4, ["find", "press"].contains(args[1]) else { fail("Expected a UI name pattern") }
let pattern = try NSRegularExpression(pattern: args[3])
func matches(_ value: String, _ regex: NSRegularExpression) -> Bool {
    regex.firstMatch(in: value, range: NSRange(value.startIndex..., in: value)) != nil
}
func button(_ node: AXUIElement) -> Bool {
    let role = attribute(node, kAXRoleAttribute) as? String ?? ""
    return [kAXButtonRole, kAXRadioButtonRole, "AXTab"].contains(role)
        && (attribute(node, kAXEnabledAttribute) as? Bool ?? true)
}
if args.contains("--confirmation") {
    // Restrict consent to the row that contains the real Later button. The Help
    // page has another identically named install button behind the modal.
    let later = try NSRegularExpression(pattern: "^(?:稍后再说|Later)$")
    let candidates = nodes.filter { button($0) && strings($0).contains { matches($0, later) } }
    guard candidates.count == 1,
          let parent = attribute(candidates[0], kAXParentAttribute),
          CFGetTypeID(parent) == AXUIElementGetTypeID() else { emit(["found": false]); exit(0) }
    nodes = walk(unsafeBitCast(parent, to: AXUIElement.self))
}
let textOnly = args.contains("--text")
let matchesFound = nodes.filter { node in
    (textOnly || button(node)) && strings(node).contains { matches($0, pattern) }
}
if matchesFound.isEmpty { emit(["found": false]); exit(0) }
if args[1] == "press" {
    guard matchesFound.count == 1 else { fail("Ambiguous accessibility action: " + args[3]) }
    guard visible else { fail("The owned application has no visible main window") }
    _ = app.activate(options: [])
    _ = AXUIElementPerformAction(matchesFound[0], "AXScrollToVisible" as CFString)
    guard AXUIElementPerformAction(matchesFound[0], kAXPressAction as CFString) == .success else {
        fail("The matching control did not accept its native accessibility press")
    }
}
emit(["found": true, "count": matchesFound.count])
