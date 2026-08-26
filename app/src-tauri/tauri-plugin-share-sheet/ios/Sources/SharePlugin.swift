import Tauri
import UIKit

struct ShareFileArgs: Decodable {
    let fileName: String
    let contents: String
    let mimeType: String
}

struct ShareTextArgs: Decodable {
    let text: String
}

@objc public class SharePlugin: Plugin {

    @objc public func shareFile(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(ShareFileArgs.self)

        let tmpDir = FileManager.default.temporaryDirectory
        let fileURL = tmpDir.appendingPathComponent(args.fileName)

        do {
            try args.contents.write(to: fileURL, atomically: true, encoding: .utf8)
        } catch {
            invoke.reject("Failed to write file for sharing: \(error)")
            return
        }

        DispatchQueue.main.async {
            guard let viewController = self.manager.viewController else {
                invoke.reject("No view controller available to present the share sheet")
                return
            }

            let activityVC = UIActivityViewController(activityItems: [fileURL], applicationActivities: nil)
            UIUtils.centerPopover(rootViewController: viewController, popoverController: activityVC)

            viewController.present(activityVC, animated: true) {
                invoke.resolve()
            }
        }
    }

    @objc public func shareText(_ invoke: Invoke) throws {
        let args = try invoke.parseArgs(ShareTextArgs.self)

        // Sharing an actual URL object (rather than a plain String) gets
        // native link-preview behavior in Messages/Mail and a clean "Copy"
        // action - falls back to plain text if it doesn't parse as a URL.
        let item: Any = URL(string: args.text) ?? args.text

        DispatchQueue.main.async {
            guard let viewController = self.manager.viewController else {
                invoke.reject("No view controller available to present the share sheet")
                return
            }

            let activityVC = UIActivityViewController(activityItems: [item], applicationActivities: nil)
            UIUtils.centerPopover(rootViewController: viewController, popoverController: activityVC)

            viewController.present(activityVC, animated: true) {
                invoke.resolve()
            }
        }
    }
}

@_cdecl("init_plugin_share_sheet")
func initPlugin() -> Plugin {
    return SharePlugin()
}
